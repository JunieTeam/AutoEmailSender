//! Cloud backup operations; restoring is a host command, as it restarts the app.

use crate::backup::cloud::{self, Enable, Enabled, Server, Status};
use crate::{CallError, Context, Nothing};

pub async fn get_cloud_backup(ctx: &Context, _: Nothing) -> Result<Status, CallError> {
    Ok(cloud::status(ctx).await?)
}

pub async fn get_cloud_backup_server(ctx: &Context, _: Nothing) -> Result<Server, CallError> {
    Ok(cloud::server(ctx, crate::now()).await?)
}

pub async fn enable_cloud_backup(ctx: &Context, input: Enable) -> Result<Enabled, CallError> {
    Ok(cloud::enable(ctx, input, crate::now()).await?)
}

pub async fn disable_cloud_backup(ctx: &Context, _: Nothing) -> Result<(), CallError> {
    Ok(cloud::disable(&ctx.db).await?)
}

pub async fn back_up_to_cloud(ctx: &Context, _: Nothing) -> Result<(), CallError> {
    Ok(cloud::back_up(ctx, crate::now()).await?)
}

pub async fn delete_cloud_backup(ctx: &Context, _: Nothing) -> Result<(), CallError> {
    Ok(cloud::delete(ctx).await?)
}

impl From<cloud::Error> for CallError {
    fn from(error: cloud::Error) -> Self {
        match error {
            cloud::Error::Account(error) => error.into(),
            cloud::Error::Local(error) => error.into(),
            cloud::Error::Database(_) => CallError::internal(error),
            cloud::Error::Io(_) => CallError::external(error.failure(), error),
            error => CallError::invalid(error.failure().message).logging(error),
        }
    }
}
