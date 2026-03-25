extern crate modkit_canonical_errors;

modkit_canonical_errors::resource_error!(UserResourceError, "gts.cf.core.users.user.v1~");

fn main() {
    // not_found requires .with_resource() before .create()
    let _err = UserResourceError::not_found("User not found").create();
}
