extern crate modkit_canonical_errors;

modkit_canonical_errors::resource_error!(UserResourceError, "gts.cf.core.users.user.v1~");

fn main() {
    // failed_precondition requires at least one .with_precondition_violation() before .create()
    let _err = UserResourceError::failed_precondition().create();
}
