//! The membership of an account in the local Administrators group (ADR-0048, threat T20).
//!
//! The service runs as SYSTEM and removes an access entry of a local administrator, so it tells a
//! local administrator apart from every other trustee. The Authorization API builds the group list
//! of the account, so a nested group counts too, and a local account needs no domain controller.
//!
//! Every other case is a refusal: a group, a service or a capability identifier, an account that
//! no longer resolves, a domain account while the domain controller is unreachable, and an account
//! of another identity provider. The caller then keeps its refusal of the directory.

use windows_sys::Win32::Foundation::LUID;
use windows_sys::Win32::Security::Authorization::{
    AUTHZ_CLIENT_CONTEXT_HANDLE, AUTHZ_RESOURCE_MANAGER_HANDLE, AUTHZ_RM_FLAG_NO_AUDIT,
    AuthzContextInfoGroupsSids, AuthzFreeContext, AuthzFreeResourceManager,
    AuthzGetInformationFromContext, AuthzInitializeContextFromSid, AuthzInitializeResourceManager,
};
use windows_sys::Win32::Security::{
    EqualSid, LookupAccountSidW, PSID, SID_AND_ATTRIBUTES, SidTypeUser, TOKEN_GROUPS,
};

use super::{Sid, sid_from_string};

/// The well-known identifier of BUILTIN\Administrators.
const ADMINISTRATORS: &str = "S-1-5-32-544";

/// A user account that is a member of the local Administrators group.
#[derive(Debug)]
pub(crate) struct Account {
    pub(crate) name: String,
    pub(crate) sid: String,
}

/// Returns the account when `sid` is a user account and a member of the local Administrators
/// group, also through a nested group. Any other account, and any error, gives `None`.
pub(crate) fn local_administrator(sid: PSID) -> Option<Account> {
    let name = user_name(sid)?;
    let text = Sid::copy(sid).ok()?.text()?;
    if !in_administrators(sid)? {
        return None;
    }
    Some(Account { name, sid: text })
}

/// The domain and the account name, for a user account only.
fn user_name(sid: PSID) -> Option<String> {
    let mut name = [0u16; 256];
    let mut domain = [0u16; 256];
    let mut name_length = u32::try_from(name.len()).ok()?;
    let mut domain_length = u32::try_from(domain.len()).ok()?;
    let mut kind = 0;
    // SAFETY: both buffers are locals of the lengths that are passed, and the identifier is valid.
    let ok = unsafe {
        LookupAccountSidW(
            std::ptr::null(),
            sid,
            name.as_mut_ptr(),
            &mut name_length,
            domain.as_mut_ptr(),
            &mut domain_length,
            &mut kind,
        )
    };
    if ok == 0 || kind != SidTypeUser {
        return None;
    }
    let name = String::from_utf16_lossy(&name[..name_length as usize]);
    let domain = String::from_utf16_lossy(&domain[..domain_length as usize]);
    Some(if domain.is_empty() {
        name
    } else {
        format!("{domain}\\{name}")
    })
}

/// The resource manager of the group list, released on drop.
struct ResourceManager(AUTHZ_RESOURCE_MANAGER_HANDLE);

impl Drop for ResourceManager {
    fn drop(&mut self) {
        // SAFETY: the manager comes from `AuthzInitializeResourceManager` and is freed once.
        unsafe { AuthzFreeResourceManager(self.0) };
    }
}

/// Holds the group list of one account, released on drop.
struct Context(AUTHZ_CLIENT_CONTEXT_HANDLE);

impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: the context comes from `AuthzInitializeContextFromSid` and is freed once.
        unsafe { AuthzFreeContext(self.0) };
    }
}

fn in_administrators(sid: PSID) -> Option<bool> {
    let administrators = sid_from_string(ADMINISTRATORS)?;
    let mut manager: AUTHZ_RESOURCE_MANAGER_HANDLE = std::ptr::null_mut();
    // SAFETY: the flag allows no callback, and the name of the manager can be null.
    let ok = unsafe {
        AuthzInitializeResourceManager(
            AUTHZ_RM_FLAG_NO_AUDIT,
            None,
            None,
            None,
            std::ptr::null(),
            &mut manager,
        )
    };
    if ok == 0 || manager.is_null() {
        return None;
    }
    let manager = ResourceManager(manager);
    let mut context: AUTHZ_CLIENT_CONTEXT_HANDLE = std::ptr::null_mut();
    let identifier = LUID {
        LowPart: 0,
        HighPart: 0,
    };
    // SAFETY: the manager is open, and a null expiration, a zero identifier and no dynamic
    // arguments are allowed.
    let ok = unsafe {
        AuthzInitializeContextFromSid(
            0,
            sid,
            manager.0,
            std::ptr::null(),
            identifier,
            std::ptr::null(),
            &mut context,
        )
    };
    if ok == 0 || context.is_null() {
        return None;
    }
    let context = Context(context);
    let mut size = 0;
    // SAFETY: a null buffer with no size asks only for the necessary size.
    unsafe {
        AuthzGetInformationFromContext(
            context.0,
            AuthzContextInfoGroupsSids,
            0,
            &mut size,
            std::ptr::null_mut(),
        );
    }
    if size < std::mem::size_of::<u32>() as u32 {
        return None;
    }
    // A token groups buffer is a count and an array of pairs of a pointer and a flag.
    let mut buffer = vec![0u64; (size as usize).div_ceil(std::mem::size_of::<u64>())];
    let bytes = u32::try_from(buffer.len() * std::mem::size_of::<u64>()).ok()?;
    // SAFETY: the buffer is aligned for the structure and holds `bytes` bytes.
    let ok = unsafe {
        AuthzGetInformationFromContext(
            context.0,
            AuthzContextInfoGroupsSids,
            bytes,
            &mut size,
            buffer.as_mut_ptr().cast(),
        )
    };
    if ok == 0 {
        return None;
    }
    let offset = std::mem::offset_of!(TOKEN_GROUPS, Groups);
    // SAFETY: the call filled at least the count at the start of the buffer.
    let count = unsafe { std::ptr::read(buffer.as_ptr().cast::<u32>()) };
    let needed = offset + usize::try_from(count).ok()? * std::mem::size_of::<SID_AND_ATTRIBUTES>();
    if (size as usize) < needed {
        return None;
    }
    let first = buffer
        .as_ptr()
        .cast::<u8>()
        .wrapping_add(offset)
        .cast::<SID_AND_ATTRIBUTES>();
    for index in 0..count {
        // SAFETY: the size check above covers each entry of the array.
        let group = unsafe { *first.add(index as usize) };
        // SAFETY: both pointers are valid security identifiers.
        if unsafe { EqualSid(group.Sid, administrators.0) } != 0 {
            return Some(true);
        }
    }
    Some(false)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::winsec::tests::current_user_sid;

    #[test]
    fn adr_0048_the_runner_account_is_a_local_administrator() {
        let text = current_user_sid().expect("cannot read the user SID");
        let sid = sid_from_string(&text).expect("cannot build the SID");
        let account = local_administrator(sid.0).expect("the runner is a local administrator");
        assert_eq!(account.sid, text);
        assert!(!account.name.is_empty(), "{account:?}");
    }

    #[test]
    fn adr_0048_builtin_users_is_not_an_administrator() {
        let sid = sid_from_string("S-1-5-32-545").expect("cannot build the SID");
        assert!(
            local_administrator(sid.0).is_none(),
            "a group is not a user account"
        );
    }

    #[test]
    fn adr_0048_a_group_in_the_administrators_group_is_not_a_user_account() {
        let sid = sid_from_string(ADMINISTRATORS).expect("cannot build the SID");
        assert!(local_administrator(sid.0).is_none());
    }

    #[test]
    fn adr_0048_an_unresolved_sid_is_not_an_administrator() {
        let sid = sid_from_string("S-1-5-21-1-2-3-4242").expect("cannot build the SID");
        assert!(local_administrator(sid.0).is_none());
    }
}
