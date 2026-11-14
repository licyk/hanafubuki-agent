//! Read only the loaded image's bounded PE metadata. Installed runtimes are not modified.
use super::memory;
use windows_sys::Win32::Foundation::HANDLE;

pub(super) struct Image {
    pub base: usize,
    pub entry: usize,
    size: usize,
    import: usize,
}
impl Image {
    pub fn msys(process: HANDLE, base: usize) -> Result<Option<Self>, String> {
        let dos = memory::read(process, base, 64)?;
        if &dos[..2] != b"MZ" {
            return Ok(None);
        }
        let offset = u32_at(&dos, 60) as usize;
        if offset > 1024 * 1024 {
            return Ok(None);
        }
        let pe = memory::read(process, base + offset, 264)?;
        if &pe[..4] != b"PE\0\0" || pe[4..6] != [0x64, 0x86] || pe[24..26] != [0x0b, 0x02] {
            return Ok(None);
        }
        let image = Self {
            base,
            entry: base + u32_at(&pe, 40) as usize,
            size: u32_at(&pe, 80) as usize,
            import: u32_at(&pe, 144) as usize,
        };
        let export = u32_at(&pe, 136) as usize;
        if export == 0 {
            return Ok(None);
        }
        let directory = image.read(process, export, 40)?;
        let name = image.string(process, u32_at(&directory, 12) as usize)?;
        if !name.eq_ignore_ascii_case("msys-2.0.dll") {
            return Ok(None);
        }
        image.read(process, image.entry - base, 1)?;
        Ok(Some(image))
    }
    pub fn acl_import(&self, process: HANDLE) -> Result<usize, String> {
        for i in 0..1024 {
            let descriptor = self.read(process, self.import + i * 20, 20)?;
            let name = u32_at(&descriptor, 12) as usize;
            if name == 0 {
                break;
            }
            if !self
                .string(process, name)?
                .eq_ignore_ascii_case("ntdll.dll")
            {
                continue;
            }
            let lookup = u32_at(&descriptor, 0) as usize;
            let table = u32_at(&descriptor, 16) as usize;
            for j in 0..4096 {
                let entry = self.read(process, lookup + j * 8, 8)?;
                let name = u64::from_le_bytes(entry.try_into().map_err(|_| "MSYS import entry")?);
                if name == 0 {
                    break;
                }
                if name & (1 << 63) != 0 {
                    continue;
                }
                if self.string(process, name as usize + 2)? == "RtlAddAccessAllowedAce" {
                    self.read(process, table + j * 8, 8)?;
                    return Ok(self.base + table + j * 8);
                }
            }
        }
        Err("MSYS runtime has no supported ACL import; command refused".into())
    }
    fn read(&self, process: HANDLE, offset: usize, size: usize) -> Result<Vec<u8>, String> {
        if offset.checked_add(size).is_none_or(|end| end > self.size) {
            return Err("MSYS image metadata is outside its mapping".into());
        }
        memory::read(process, self.base + offset, size)
    }
    fn string(&self, process: HANDLE, offset: usize) -> Result<String, String> {
        let bytes = self.read(process, offset, 256.min(self.size.saturating_sub(offset)))?;
        let end = bytes
            .iter()
            .position(|b| *b == 0)
            .ok_or("MSYS image name is too long")?;
        String::from_utf8(bytes[..end].to_vec()).map_err(|_| "invalid MSYS image name".into())
    }
}
fn u32_at(bytes: &[u8], index: usize) -> u32 {
    u32::from_le_bytes([
        bytes[index],
        bytes[index + 1],
        bytes[index + 2],
        bytes[index + 3],
    ])
}
