//! Fixture for scripts/check-comments.sh. Every comment below must be flagged.

/// The user.
pub struct User;

/// Creates a tenant.
pub fn create_tenant() {}

/// The role of the user in the tenant.
pub struct Membership;

// A field from the review finding N2.
fn first() {}

// The fix for issue #17.
fn second() {}

// Now insert the row.
fn third() {}

// Then return the identifier.
fn fourth() {}

/// Lists the users of the tenant.
pub fn list_users() {}

/// The function returns the number of rows.
fn count_rows() {}

/// The caller opens the file. The caller reads the value. The caller closes the file.
fn load_value() {}
