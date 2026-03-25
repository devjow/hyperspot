extern crate modkit_canonical_errors;

modkit_canonical_errors::resource_error!(UserResourceError, "gts.cf.core.users.user.v1~");

fn main() {
    // out_of_range requires .with_field_violation() before .create()
    let _err = UserResourceError::out_of_range("Page out of range").create();
}
