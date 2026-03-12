extern crate cf_modkit_errors;

fn main() {
    // aborted requires .with_reason() before .create()
    cf_modkit_errors::resource_error!(UserResourceError, "gts.cf.core.users.user.v1~");

    let _err = UserResourceError::aborted("Operation aborted due to concurrency conflict").create();
}
