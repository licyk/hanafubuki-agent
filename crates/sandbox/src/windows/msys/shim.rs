use windows_sys::Win32::Foundation::HANDLE;

use super::{image::Image, memory};

pub(super) fn install(
    process: HANDLE,
    image: &Image,
    account: &[u8],
    logon: &[u8],
) -> Result<(), String> {
    let slot = image.acl_import(process)?;
    let original = memory::read(process, slot, 8)?;
    let allocation = memory::allocate(process, 4096)?;
    let account_address = allocation + 256;
    let logon_address = account_address + account.len();
    // Windows x64 ABI: ACL, revision, mask, SID are RCX, EDX, R8D, R9.
    // Only volatile registers are touched; the original function is tail-called.
    let mut code = vec![0x41, 0x81, 0xf8, 0, 0, 0, 0x10]; // cmp r8d, GENERIC_ALL
    let mut exits = Vec::new();
    branch(&mut code, &mut exits, 0x75);
    code.extend([0x4d, 0x85, 0xc9]); // test r9,r9
    branch(&mut code, &mut exits, 0x74);
    code.extend([0x41, 0x80, 0x79, 1, account[1]]); // SID subauthority count
    branch(&mut code, &mut exits, 0x75);
    code.extend([0x49, 0xba]); // mov r10, account
    code.extend((account_address as u64).to_le_bytes());
    code.extend([0x45, 0x31, 0xdb]); // xor r11d,r11d
    let compare = code.len();
    code.extend([0x43, 0x0f, 0xb6, 0x04, 0x19]); // movzx eax,byte [r9+r11]
    code.extend([0x43, 0x3a, 0x04, 0x1a]); // cmp al,[r10+r11]
    branch(&mut code, &mut exits, 0x75);
    code.extend([0x49, 0xff, 0xc3]); // inc r11
    code.extend([0x49, 0x83, 0xfb, account.len() as u8]);
    code.extend([0x72, (compare as isize - (code.len() + 2) as isize) as u8]);
    code.extend([0x49, 0xb9]); // mov r9, logon
    code.extend((logon_address as u64).to_le_bytes());
    for offset in exits {
        code[offset] = (code.len() - offset - 1) as u8;
    }
    code.extend([0x48, 0xb8]);
    code.extend(original);
    code.extend([0xff, 0xe0]); // jmp rax
    memory::write(process, allocation, &code)?;
    memory::write(process, account_address, account)?;
    memory::write(process, logon_address, logon)?;
    memory::executable(process, allocation, 4096)?;
    memory::write(process, slot, &(allocation as u64).to_le_bytes())
}

fn branch(code: &mut Vec<u8>, exits: &mut Vec<usize>, opcode: u8) {
    code.push(opcode);
    exits.push(code.len());
    code.push(0);
}
