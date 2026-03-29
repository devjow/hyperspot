use sea_orm_migration::prelude::*;

mod m20260302_000001_initial;
mod m20260329_000001_add_last_progress_at;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260302_000001_initial::Migration),
            Box::new(m20260329_000001_add_last_progress_at::Migration),
        ]
    }
}
