use mongodb::{
    Client, ClientSession,
    options::{ReadConcern, ReadPreference, SelectionCriteria, TransactionOptions, WriteConcern},
};

pub(super) async fn run<R, F>(
    client: &Client,
    read_only: bool,
    callback: F,
) -> mongodb::error::Result<R>
where
    F: for<'session> AsyncFnMut(&'session mut ClientSession) -> mongodb::error::Result<R>,
{
    let mut session = client.start_session().await?;
    let options = TransactionOptions::builder()
        .read_concern(ReadConcern::snapshot())
        .write_concern((!read_only).then(WriteConcern::majority))
        .selection_criteria(SelectionCriteria::ReadPreference(ReadPreference::Primary))
        .build();
    session
        .start_transaction()
        .with_options(options)
        .and_run2(callback)
        .await
}
