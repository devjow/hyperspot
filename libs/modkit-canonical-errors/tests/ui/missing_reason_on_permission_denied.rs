extern crate modkit_canonical_errors;

fn main() {
    // permission_denied requires .with_reason() before .create()
    modkit_canonical_errors::resource_error!(UserResourceError, "gts.cf.core.users.user.v1~");

    let _err = UserResourceError::permission_denied().create();
}
