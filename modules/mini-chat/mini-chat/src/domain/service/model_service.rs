use std::sync::Arc;

use authz_resolver_sdk::PolicyEnforcer;
use modkit_macros::domain_model;

use crate::domain::repos::ModelPrefRepository;

use super::DbProvider;

/// Service handling model listing and selection.
#[domain_model]
pub struct ModelService {
    _db: Arc<DbProvider>,
    _model_pref_repo: Arc<dyn ModelPrefRepository>,
    _enforcer: PolicyEnforcer,
}

impl ModelService {
    pub(crate) fn new(
        db: Arc<DbProvider>,
        model_pref_repo: Arc<dyn ModelPrefRepository>,
        enforcer: PolicyEnforcer,
    ) -> Self {
        Self {
            _db: db,
            _model_pref_repo: model_pref_repo,
            _enforcer: enforcer,
        }
    }
}
