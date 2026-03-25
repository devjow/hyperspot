extern crate modkit_canonical_errors;

modkit_canonical_errors::resource_error!(UserResourceError, "gts.cf.core.users.user.v1~");

fn main() {
    // resource_exhausted requires at least one .with_quota_violation() before .create()
    let _err = UserResourceError::resource_exhausted("Quota exceeded").create();
}
