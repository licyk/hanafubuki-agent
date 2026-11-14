// Derived from Codex windows-sandbox-rs identity.rs and dpapi.rs at 57a808a392
// (Apache-2.0). Credentials stay in a protected local file, never argv or logs.

use std::ptr::{null, null_mut};

use serde::{Deserialize, Serialize};
use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS};
use windows_sys::Win32::NetworkManagement::NetManagement::{
    NERR_UserExists, NetUserAdd, NetUserSetInfo, UF_DONT_EXPIRE_PASSWD, UF_NORMAL_ACCOUNT,
    UF_PASSWD_CANT_CHANGE, USER_INFO_1, USER_INFO_1003, USER_PRIV_USER,
};
use windows_sys::Win32::Security::Cryptography::{
    BCryptGenRandom, CryptProtectData, CryptUnprotectData, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

use super::super::process::wide;
use super::super::token::last_error;

#[derive(Serialize, Deserialize)]
pub(super) struct Credentials {
    pub online: Account,
    pub offline: Account,
    pub filter_keys: [[u8; 16]; 4],
}

#[derive(Serialize, Deserialize)]
pub(super) struct Account {
    pub name: String,
    password: Vec<u8>,
}

impl Credentials {
    pub fn create() -> Result<Self, String> {
        let id = random()?;
        let prefix = format!(
            "hana-{:08x}",
            u32::from_le_bytes(id[..4].try_into().map_err(|_| "account identifier")?)
        );
        Ok(Self {
            online: Account::new(format!("{prefix}-on"))?,
            offline: Account::new(format!("{prefix}-off"))?,
            filter_keys: [random()?, random()?, random()?, random()?],
        })
    }

    pub fn keys(&self) -> [GUID; 4] {
        self.filter_keys
            .map(|key| GUID::from_u128(u128::from_be_bytes(key)))
    }
}

impl Account {
    fn new(name: String) -> Result<Self, String> {
        let mut password = format!("Ha!{:032x}a9", u128::from_le_bytes(random()?)).into_bytes();
        let encrypted = protect(&password, true);
        password.fill(0);
        Ok(Self {
            name,
            password: encrypted?,
        })
    }

    pub fn password(&self) -> Result<Password, String> {
        let mut bytes = protect(&self.password, false)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| "invalid sandbox credentials; run setup again")?;
        let password = Password(wide(text));
        bytes.fill(0);
        Ok(password)
    }

    pub fn verify(&self) -> Result<(), String> {
        use std::os::windows::io::{FromRawHandle, OwnedHandle};
        use windows_sys::Win32::Security::{
            LogonUserW, LOGON32_LOGON_INTERACTIVE, LOGON32_PROVIDER_DEFAULT,
        };
        let name = wide(&self.name);
        let domain = wide(".");
        let password = self.password()?;
        let mut token = null_mut();
        // SAFETY: the credentials live through the call; the resulting token is owned.
        if unsafe {
            LogonUserW(
                name.as_ptr(),
                domain.as_ptr(),
                password.0.as_ptr(),
                LOGON32_LOGON_INTERACTIVE,
                LOGON32_PROVIDER_DEFAULT,
                &mut token,
            )
        } == 0
        {
            return Err(last_error(
                "sandbox account logon; run /setup-default-sandbox to repair",
            ));
        }
        let _token = unsafe { OwnedHandle::from_raw_handle(token as _) };
        Ok(())
    }

    pub fn provision(&self) -> Result<(), String> {
        let name = wide(&self.name);
        let password = self.password()?;
        let comment = wide("Hanafubuki Agent restricted command identity");
        let user = USER_INFO_1 {
            usri1_name: name.as_ptr().cast_mut(),
            usri1_password: password.0.as_ptr().cast_mut(),
            usri1_password_age: 0,
            usri1_priv: USER_PRIV_USER,
            usri1_home_dir: null_mut(),
            usri1_comment: comment.as_ptr().cast_mut(),
            usri1_flags: UF_NORMAL_ACCOUNT | UF_DONT_EXPIRE_PASSWD | UF_PASSWD_CANT_CHANGE,
            usri1_script_path: null_mut(),
        };
        // SAFETY: all strings and the structure live through this synchronous call.
        let code =
            unsafe { NetUserAdd(null(), 1, (&user as *const USER_INFO_1).cast(), null_mut()) };
        if code == NERR_UserExists {
            // Setup repairs a password changed since the encrypted record was saved.
            let info = USER_INFO_1003 {
                usri1003_password: password.0.as_ptr().cast_mut(),
            };
            let code = unsafe {
                NetUserSetInfo(
                    null(),
                    name.as_ptr(),
                    1003,
                    (&info as *const USER_INFO_1003).cast(),
                    null_mut(),
                )
            };
            if code != ERROR_SUCCESS {
                return Err(format!("repair sandbox account: Windows error {code}"));
            }
        } else if code != ERROR_SUCCESS {
            return Err(format!("create sandbox account: Windows error {code}"));
        }
        Ok(())
    }
}

pub(super) struct Password(pub Vec<u16>);

impl Drop for Password {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

pub(super) fn random() -> Result<[u8; 16], String> {
    let mut bytes = [0; 16];
    // SAFETY: BCrypt writes exactly the supplied buffer length.
    let status = unsafe {
        BCryptGenRandom(
            null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        Err("cannot generate sandbox credentials".into())
    } else {
        Ok(bytes)
    }
}

fn protect(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
    // Machine scope lets UAC run under another administrator. The file's protected
    // DACL grants only the requesting user, SYSTEM and administrators access.
    let ok = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                null(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                null_mut(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 {
        return Err(last_error("sandbox credential protection"));
    }
    // SAFETY: DPAPI allocated cbData bytes, which are copied then cleared and freed.
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        std::ptr::write_bytes(output.pbData, 0, output.cbData as usize);
        LocalFree(output.pbData.cast());
    }
    Ok(result)
}
