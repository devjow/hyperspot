use std::sync::Arc;

use authz_resolver_sdk::PolicyEnforcer;
use modkit_macros::domain_model;

use crate::domain::repos::QuotaUsageRepository;

use super::DbProvider;

/// Service handling quota tracking and enforcement.
#[domain_model]
pub struct QuotaService {
    _db: Arc<DbProvider>,
    _repo: Arc<dyn QuotaUsageRepository>,
    _enforcer: PolicyEnforcer,
}

impl QuotaService {
    pub(crate) fn new(
        db: Arc<DbProvider>,
        repo: Arc<dyn QuotaUsageRepository>,
        enforcer: PolicyEnforcer,
    ) -> Self {
        Self {
            _db: db,
            _repo: repo,
            _enforcer: enforcer,
        }
    }
}
