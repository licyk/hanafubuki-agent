param(
    [string]$Version = 'latest',
    [ValidateSet('auto', 'x64', 'arm64')][string]$Architecture = 'auto',
    [string]$InstallDir = $env:HANAFUBUKI_AGENT_INSTALL_DIR,
    [switch]$NoPathUpdate
)

function Install-HanafubukiAgent {
    param(
        [string]$Version = 'latest',
        [ValidateSet('auto', 'x64', 'arm64')][string]$Architecture = 'auto',
        [string]$InstallDir = $env:HANAFUBUKI_AGENT_INSTALL_DIR,
        [switch]$NoPathUpdate,
        [string]$ReleaseApi = $(if ($env:HANAFUBUKI_AGENT_RELEASE_API_URL) { $env:HANAFUBUKI_AGENT_RELEASE_API_URL } else { 'https://api.github.com/repos/licyk/hanafubuki-agent/releases' }),
        [string]$EnvironmentKey = 'Environment'
    )
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    $windows = [Environment]::OSVersion.Platform -eq 'Win32NT'
    $platform = if ($windows) { 'pc-windows-msvc' }
        elseif ($IsLinux) { 'unknown-linux-gnu' }
        elseif ($IsMacOS) { 'apple-darwin' }
        else { throw 'Only Windows, Linux and macOS are supported.' }
    if ($Architecture -eq 'auto') {
        $native = if ($windows) {
            (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment').PROCESSOR_ARCHITECTURE
        } else { [Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString() }
        $Architecture = switch ($native) {
            'AMD64' { 'x64' }
            'X64' { 'x64' }
            'ARM64' { 'arm64' }
            default { throw "Unsupported architecture: $native" }
        }
    }
    if (!$InstallDir) {
        $InstallDir = if ($windows) { Join-Path $env:LOCALAPPDATA 'hanafubuki-agent\bin' }
            else { Join-Path $HOME '.local/bin' }
    }
    $InstallDir = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($InstallDir)
    if ($InstallDir.Contains([IO.Path]::PathSeparator) -or $InstallDir.Contains("`n") -or $InstallDir.Contains("`r")) {
        throw 'The installation directory cannot contain a PATH separator or newline.'
    }
    $target = $(if ($Architecture -eq 'arm64') { 'aarch64' } else { 'x86_64' }) + "-$platform"
    $extension = if ($windows) { 'zip' } else { 'tar.gz' }
    $binary = if ($windows) { 'hanafubuki-agent.exe' } else { 'hanafubuki-agent' }
    $suffix = 'latest'
    if ($Version -ne 'latest') {
        $Version = $Version -creplace '^v', ''
        if ($Version -cnotmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$') { throw 'Expected a release version such as 0.4.3.' }
        $suffix = 'tags/' + [Uri]::EscapeDataString("v$Version")
    }
    $oldTls = [Net.ServicePointManager]::SecurityProtocol
    $staging = $null
    try {
        [Net.ServicePointManager]::SecurityProtocol = $oldTls -bor [Net.SecurityProtocolType]::Tls12
        $headers = @{ Accept = 'application/vnd.github+json'; 'User-Agent' = 'hanafubuki-agent-installer' }
        if ($env:GITHUB_TOKEN) { $headers.Authorization = "Bearer $env:GITHUB_TOKEN" }
        $release = Invoke-RestMethod -Uri "$ReleaseApi/$suffix" -Headers $headers -TimeoutSec 60
        $tag = [string]$release.tag_name
        if ($release.draft -or ($Version -eq 'latest' -and $release.prerelease) -or
            $tag -cnotmatch '^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$') { throw 'The release has no usable version.' }
        if ($Version -ne 'latest' -and $tag -cne "v$Version") { throw 'The release differs from the requested version.' }
        $Version = $tag.Substring(1)
        $directory = "hanafubuki-agent-$Version-$target"
        $name = "$directory.$extension"
        $asset = @($release.assets | Where-Object { $_.name -ceq $name })
        $checksum = @($release.assets | Where-Object { $_.name -ceq "$name.sha256" })
        if ($asset.Count -ne 1 -or $checksum.Count -ne 1) { throw "Release $tag has no unique $name and checksum." }
        $staging = Join-Path $InstallDir ('.hanafubuki-agent-install-' + [Guid]::NewGuid().ToString('N'))
        $null = [IO.Directory]::CreateDirectory($staging)
        $archive = Join-Path $staging $name
        $hashFile = Join-Path $staging 'checksum'
        Write-Host "Downloading hanafubuki-agent $Version ($target)..."
        Save-AgentDownload $asset[0].browser_download_url $archive
        Save-AgentDownload $checksum[0].browser_download_url $hashFile
        $expected = ([IO.File]::ReadAllText($hashFile).Trim() -split '\s+')[0]
        $sha256 = [Security.Cryptography.SHA256]::Create()
        $archiveStream = [IO.File]::OpenRead($archive)
        try { $actual = [BitConverter]::ToString($sha256.ComputeHash($archiveStream)).Replace('-', '') }
        finally { $archiveStream.Dispose(); $sha256.Dispose() }
        if ($expected -notmatch '^[a-fA-F0-9]{64}$' -or $actual -ine $expected) { throw 'Downloaded archive SHA-256 does not match.' }
        if ($windows) {
            Add-Type -AssemblyName System.IO.Compression.FileSystem
            [IO.Compression.ZipFile]::ExtractToDirectory($archive, $staging)
        } else {
            & tar xzf $archive -C $staging "$directory/$binary" "$directory/LICENSE" "$directory/NOTICE" "$directory/THIRD-PARTY-LICENSES.txt"
            if ($LASTEXITCODE -ne 0) { throw 'Could not extract the release archive.' }
        }
        $unpacked = Join-Path $staging $directory
        $download = Join-Path $unpacked $binary
        if (!$windows) {
            & chmod 755 $download
            if ($LASTEXITCODE -ne 0) { throw 'Could not make the downloaded binary executable.' }
        }
        $reported = & $download --version
        if ($LASTEXITCODE -ne 0 -or "$reported".Trim() -cne "hanafubuki-agent $Version") { throw 'Downloaded executable reports an unexpected version.' }
        $notices = Join-Path $InstallDir 'hanafubuki-agent-notices'
        $null = [IO.Directory]::CreateDirectory($notices)
        foreach ($file in @('LICENSE', 'NOTICE', 'THIRD-PARTY-LICENSES.txt')) {
            [IO.File]::Copy((Join-Path $unpacked $file), (Join-Path $notices $file), $true)
        }
        $executable = Join-Path $InstallDir $binary
        if (Test-Path -LiteralPath $executable) {
            [IO.File]::Replace($download, $executable, [NullString]::Value)
        } else { [IO.File]::Move($download, $executable) }
        if (!$NoPathUpdate) {
            if ($windows) { Add-AgentPath $InstallDir $EnvironmentKey }
            else { Add-AgentPowerShellPath $InstallDir }
        }
        Write-Host "Installed $executable"
        Write-Host 'Open a new terminal, then run hanafubuki-agent. Update with hanafubuki-agent update.'
    } finally {
        [Net.ServicePointManager]::SecurityProtocol = $oldTls
        if ($staging -and (Test-Path -LiteralPath $staging)) { Remove-Item -LiteralPath $staging -Recurse -Force }
    }
}

function Save-AgentDownload {
    param([string]$Url, [string]$Path)
    # Windows PowerShell's -OutFile treats brackets as wildcards.
    $response = Invoke-WebRequest -UseBasicParsing -Uri $Url -TimeoutSec 600
    $file = [IO.File]::Create($Path)
    try { $response.RawContentStream.CopyTo($file) }
    finally { $file.Dispose(); $response.RawContentStream.Dispose() }
}

function Add-AgentPath {
    param([string]$Directory, [string]$EnvironmentKey)
    $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($EnvironmentKey)
    try {
        $path = [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        $kind = [Microsoft.Win32.RegistryValueKind]::ExpandString
        if ($key.GetValueNames() -contains 'Path') { $kind = $key.GetValueKind('Path') }
        $matches = @($path -split ';' | Where-Object { [Environment]::ExpandEnvironmentVariables($_).Trim('"').TrimEnd('\') -ieq $Directory.TrimEnd('\') })
        if ($matches.Count -eq 0) {
            $updated = if ($path) { "$Directory;$path" } else { $Directory }
            $key.SetValue('Path', $updated, $kind)
        }
    } finally { $key.Dispose() }
    if (@($env:PATH -split ';' | Where-Object { $_.TrimEnd('\') -ieq $Directory.TrimEnd('\') }).Count -eq 0) { $env:PATH = "$Directory;$env:PATH" }
    if (!('HanafubukiAgent.InstallerEnvironment' -as [type])) {
        Add-Type @'
using System;
using System.Runtime.InteropServices;
namespace HanafubukiAgent {
    public static class InstallerEnvironment {
        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        public static extern IntPtr SendMessageTimeout(IntPtr window, uint message,
            UIntPtr wParam, string lParam, uint flags, uint timeout, out UIntPtr result);
    }
}
'@
    }
    $result = [UIntPtr]::Zero
    $null = [HanafubukiAgent.InstallerEnvironment]::SendMessageTimeout([IntPtr]0xffff, 0x1a, [UIntPtr]::Zero, 'Environment', 2, 1000, [ref]$result)
}

function Add-AgentPowerShellPath {
    param([string]$Directory)
    $profilePath = $PROFILE.CurrentUserAllHosts
    $quoted = $Directory.Replace("'", "''")
    $line = 'if (($env:PATH -split '':'') -cnotcontains ''' + $quoted + ''') { $env:PATH = ''' + $quoted + ':'' + $env:PATH } # hanafubuki-agent'
    $null = [IO.Directory]::CreateDirectory((Split-Path -Parent $profilePath))
    $lines = if (Test-Path -LiteralPath $profilePath) { [IO.File]::ReadAllLines($profilePath) } else { @() }
    if ($lines -cnotcontains $line) { [IO.File]::AppendAllText($profilePath, "`n$line`n") }
    if (($env:PATH -split ':') -cnotcontains $Directory) { $env:PATH = "${Directory}:$env:PATH" }
    Write-Host "PATH configured in $profilePath."
}

if ($MyInvocation.InvocationName -ne '.') { Install-HanafubukiAgent @PSBoundParameters }
