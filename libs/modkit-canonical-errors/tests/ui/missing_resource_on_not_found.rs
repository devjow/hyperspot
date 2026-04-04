extern crate modkit_canonical_errors;

use modkit_canonical_errors::resource_error;

#[resource_error("gts.cf.core.users.user.v1~")]
struct UserResourceError;

fn main() {
    // not_found requires .with_resource() before .create()
    let _err = UserResourceError::not_found("User not found").create();
}
