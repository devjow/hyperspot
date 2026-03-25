extern crate modkit_canonical_errors;

modkit_canonical_errors::resource_error!(UserResourceError, "gts.cf.core.users.user.v1~");

fn main() {
    // invalid_argument requires at least one .with_field_violation() before .create()
    let _err = UserResourceError::invalid_argument().create();
}
