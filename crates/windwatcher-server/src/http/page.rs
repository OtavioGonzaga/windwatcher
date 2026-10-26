use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub(super) struct PageResponse<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

impl<S, T: From<S>> From<windwatcher::Page<S>> for PageResponse<T> {
    fn from(page: windwatcher::Page<S>) -> Self {
        let next_cursor = page.next_cursor().map(str::to_owned);
        Self {
            has_more: next_cursor.is_some(),
            next_cursor,
            items: page.into_items().into_iter().map(T::from).collect(),
        }
    }
}
