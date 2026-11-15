// Run the actual installers against release archives served by a local HTTP server.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFile } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdtemp, mkdir, writeFile, readFile, copyFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

const exec = promisify(execFile);
const repo = fileURLToPath(new URL('../..', import.meta.url));
const windows = process.platform === 'win32';
const arg = (name) => {
  const i = process.argv.indexOf(name);
  return i === -1 ? undefined : process.argv[i + 1];
};
const nativeArch = windows ? (process.env.PROCESSOR_ARCHITEW6432 || process.env.PROCESSOR_ARCHITECTURE) : process.arch;
const arm = /arm64/i.test(nativeArch);
const target = arg('--target') || `${arm ? 'aarch64' : 'x86_64'}-${windows ? 'pc-windows-msvc' : process.platform === 'darwin' ? 'apple-darwin' : 'unknown-linux-gnu'}`;
const root = await mkdtemp(path.join(tmpdir(), 'hana-installer-'));
const home = path.join(root, 'home');
const dest = path.join(root, "path with spaces [probe]' 中", 'bin');
const executable = windows ? 'hanafubuki-agent.exe' : 'hanafubuki-agent';
let server;
try {
  await mkdir(home);
  let binary = arg('--binary');
  if (binary) binary = path.resolve(binary);
  else {
    binary = path.join(root, executable);
    const source = path.join(root, 'fixture.rs');
    await writeFile(source, 'fn main() { println!("hanafubuki-agent 1.2.3"); }\n');
    await exec('rustc', ['--crate-name', 'installer_fixture', source, '-o', binary]);
  }
  const { stdout } = await exec(binary, ['--version']);
  const version = stdout.trim().replace(/^hanafubuki-agent /, '');
  assert.match(version, /^\d+\.\d+\.\d+$/);
  const directory = `hanafubuki-agent-${version}-${target}`;
  const unpacked = path.join(root, directory);
  await mkdir(unpacked);
  await copyFile(binary, path.join(unpacked, executable));
  for (const name of ['LICENSE', 'NOTICE', 'THIRD-PARTY-LICENSES.txt']) await writeFile(path.join(unpacked, name), `fixture ${name}\n`);
  const archiveName = `${directory}.${windows ? 'zip' : 'tar.gz'}`;
  const archive = path.join(root, archiveName);
  const quote = (value) => `'${value.replaceAll("'", "''")}'`;
  if (windows) {
    await exec('powershell.exe', ['-NoProfile', '-Command', `Add-Type -AssemblyName System.IO.Compression.FileSystem; [IO.Compression.ZipFile]::CreateFromDirectory(${quote(unpacked)}, ${quote(archive)}, 'Optimal', $true)`]);
  } else await exec('tar', ['czf', archive, '-C', root, directory]);
  const content = await readFile(archive);
  const digest = createHash('sha256').update(content).digest('hex');
  let corrupt = false;
  let base;
  const downloads = [];
  server = createServer((req, res) => {
    const url = decodeURIComponent(req.url);
    if (url === '/releases/latest') { res.writeHead(302, { Location: `${base}/releases/tag/v${version}` }); res.end(); }
    else if (url === `/releases/tag/v${version}`) res.end('release');
    else if (url === '/api/latest' || url === `/api/tags/v${version}`) {
      res.setHeader('Content-Type', 'application/json');
      res.end(JSON.stringify({ tag_name: `v${version}`, draft: false, prerelease: false,
        assets: [archiveName, `${archiveName}.sha256`].map(name => ({ name, browser_download_url: `${base}/releases/download/v${version}/${name}` })) }));
    } else if (url === `/releases/download/v${version}/${archiveName}`) { downloads.push(archiveName); res.end(content); }
    else if (url === `/releases/download/v${version}/${archiveName}.sha256`) res.end(`${corrupt ? '0'.repeat(64) : digest}  ${archiveName}\n`);
    else { res.statusCode = 404; res.end('not found'); }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  base = `http://127.0.0.1:${server.address().port}`;
  const environmentKey = `Software\\HanafubukiAgentInstallerTests\\${process.pid}`;
  const psProfile = path.join(home, 'powershell', 'profile.ps1');
  const shells = windows ? ['powershell.exe', 'pwsh.exe', ...(arg('--bash') ? [arg('--bash')] : [])] : ['bash', 'pwsh'];
  for (const installerShell of shells) {
    await rm(dest, { recursive: true, force: true });
    const bashOnWindows = windows && installerShell === arg('--bash');
    const run = async ({ version: requested = 'latest', noPath = true } = {}) => {
      if (bashOnWindows) {
        const command = 'export PATH=/usr/bin:/bin; export HOME=$(/usr/bin/cygpath -au -- "$1"); script=$(/usr/bin/cygpath -au -- "$2"); shift 2; bash "$script" "$@"';
        return exec(installerShell, ['-c', command, 'test', home, path.join(repo, 'install.sh'), '--version', requested, '--install-dir', dest, ...(noPath ? ['--no-path-update'] : [])], {
          env: { ...process.env, GITHUB_TOKEN: '', HANAFUBUKI_AGENT_RELEASE_API_URL: base + '/api' },
        });
      }
      if (installerShell !== 'bash') {
        const command = `$ErrorActionPreference = 'Stop'; $PROFILE = [pscustomobject]@{ CurrentUserAllHosts = ${quote(psProfile)} }; . ${quote(path.join(repo, 'install.ps1'))}; Install-HanafubukiAgent -Version ${quote(requested)} -InstallDir ${quote(dest)} -ReleaseApi ${quote(`${base}/api`)} -EnvironmentKey ${quote(environmentKey)} ${noPath ? '-NoPathUpdate' : ''}`;
        return exec(installerShell, ['-NoProfile', '-NonInteractive', '-Command', command], { env: { ...process.env, GITHUB_TOKEN: '', HOME: home } });
      }
      return exec('bash', [path.join(repo, 'install.sh'), '--version', requested, '--install-dir', dest, ...(noPath ? ['--no-path-update'] : [])], {
        env: { ...process.env, HOME: home, SHELL: '/bin/bash', HANAFUBUKI_AGENT_INSTALL_DIR: path.join(root, 'wrong'), HANAFUBUKI_AGENT_RELEASE_BASE_URL: `${base}/releases` },
      });
    };
    await run();
    const installed = path.join(dest, executable);
    assert.equal((await exec(installed, ['--version'])).stdout.trim(), stdout.trim());
    assert.deepEqual(await readFile(installed), await readFile(binary));
    assert.equal((await readFile(path.join(dest, 'hanafubuki-agent-notices', 'NOTICE'), 'utf8')).trim(), 'fixture NOTICE');
    console.log('ok: latest selects the native archive, installs and preserves notices (spaces, brackets, Unicode)');
    await run({ version: `v${version}` });
    assert.deepEqual(await readFile(installed), await readFile(binary));
    console.log('ok: pinned version replaces an existing installation');
    if (installerShell !== 'bash' && !bashOnWindows) {
      await exec(installerShell, ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', path.join(repo, 'install.ps1'), '-Version', version, '-NoPathUpdate'], {
        env: { ...process.env, GITHUB_TOKEN: '', HANAFUBUKI_AGENT_INSTALL_DIR: dest, HANAFUBUKI_AGENT_RELEASE_API_URL: `${base}/api` },
      });
      assert.deepEqual(await readFile(installed), await readFile(binary));
      console.log('ok: PowerShell file entry point accepts the installation directory from the environment');
    }
    corrupt = true;
    await assert.rejects(run(), /SHA-256/);
    assert.deepEqual(await readFile(installed), await readFile(binary));
    corrupt = false;
    console.log('ok: checksum failure leaves the previous executable intact');
    await assert.rejects(run({ version: 'not-a-version' }), /version/);
    console.log('ok: invalid version is refused');
    if (bashOnWindows) {
      console.log('ok: MSYS2 restricted PATH finds Windows PowerShell and installs the Windows archive');
    } else if (windows) {
      try {
        await run({ noPath: false });
        await run({ noPath: false });
        const command = `$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey(${quote(environmentKey)}); try { $value = $key.GetValue('Path'); if (@($value -split ';').Count -ne 1 -or $value -cne ${quote(dest)}) { throw 'PATH not idempotent' } } finally { $key.Dispose() }`;
        await exec('powershell.exe', ['-NoProfile', '-Command', command]);
      } finally {
        await exec('powershell.exe', ['-NoProfile', '-Command', `[Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree(${quote(environmentKey)}, $false)`]);
      }
    } else if (installerShell === 'pwsh') {
      await assert.rejects(readFile(psProfile), { code: 'ENOENT' });
      await run({ noPath: false });
      await run({ noPath: false });
      const profileText = await readFile(psProfile, 'utf8');
      assert.equal(profileText.split('# hanafubuki-agent').length - 1, 1, profileText);
      const command = `. ${quote(psProfile)}; . ${quote(psProfile)}; (Get-Command hanafubuki-agent).Source; @($env:PATH -split ':' | Where-Object { $_ -ceq ${quote(dest)} }).Count`;
      const { stdout: resolved } = await exec('pwsh', ['-NoProfile', '-Command', command]);
      assert.deepEqual(resolved.trim().split(/\r?\n/), [installed, '1']);
    } else {
      const profile = path.join(home, process.platform === 'darwin' ? '.bash_profile' : '.bashrc');
      await assert.rejects(readFile(profile), { code: 'ENOENT' });
      await run({ noPath: false });
      await run({ noPath: false });
      const profileText = await readFile(profile, 'utf8');
      assert.equal(profileText.split('# hanafubuki-agent').length - 1, 1, profileText);
      const { stdout: resolved } = await exec('bash', ['-c', 'source "$1"; command -v hanafubuki-agent', 'test', profile]);
      assert.equal(resolved.trim(), installed);
    }
    assert.ok(downloads.length >= 5);
    console.log(`ok: ${installerShell} installer checks passed`);
  }
} finally {
  if (server) await new Promise(resolve => server.close(resolve));
  await rm(root, { recursive: true, force: true });
}
