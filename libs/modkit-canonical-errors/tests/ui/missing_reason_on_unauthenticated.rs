extern crate modkit_canonical_errors;

use modkit_canonical_errors::CanonicalError;

fn main() {
    // unauthenticated requires .with_reason() before .create()
    let _err = CanonicalError::unauthenticated().create();
}
