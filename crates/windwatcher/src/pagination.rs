/// A validated number of items per page, independent of transport and resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageLimit(usize);

/// An invalid requested page size; values are never silently clamped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PageLimitError {
    /// Pages must request at least one item.
    #[error("page limit must be greater than zero")]
    Zero,
    /// The requested size exceeds the library's bound.
    #[error("page limit {requested} exceeds maximum {maximum}")]
    TooLarge {
        /// Requested number of items.
        requested: usize,
        /// Inclusive maximum.
        maximum: usize,
    },
}

impl PageLimit {
    /// Default page size: 50 items.
    pub const DEFAULT: Self = Self(50);
    /// Largest supported page size: 100 items.
    pub const MAX: Self = Self(100);

    /// Validates an inclusive size in `1..=100`.
    pub fn new(requested: usize) -> Result<Self, PageLimitError> {
        match requested {
            0 => Err(PageLimitError::Zero),
            1..=100 => Ok(Self(requested)),
            _ => Err(PageLimitError::TooLarge {
                requested,
                maximum: Self::MAX.get(),
            }),
        }
    }

    /// Returns the validated item count.
    pub const fn get(self) -> usize {
        self.0
    }
}

impl Default for PageLimit {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A next-only page request. Cursors must be treated as uninterpreted tokens.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageRequest {
    cursor: Option<String>,
    limit: PageLimit,
}

impl PageRequest {
    /// Requests the first page with a validated size.
    pub fn new(limit: PageLimit) -> Self {
        Self {
            cursor: None,
            limit,
        }
    }
    /// Continues from an opaque token, retaining the requested page size.
    /// List use cases reject tokens above [`crate::MAX_CURSOR_BYTES`] encoded bytes
    /// before decoding; this constructor only stores the uninterpreted token.
    pub fn with_cursor(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = Some(cursor.into());
        self
    }
    /// Returns the continuation token, or `None` for the first page.
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }
    /// Returns the validated requested size.
    pub fn limit(&self) -> PageLimit {
        self.limit
    }
}

/// Resource-independent results with an optional continuation toward more items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page<T> {
    items: Vec<T>,
    next_cursor: Option<String>,
}

impl<T> Page<T> {
    pub(crate) fn new(items: Vec<T>, next_cursor: Option<String>) -> Self {
        Self { items, next_cursor }
    }
    /// Returns items in the resource's documented order.
    pub fn items(&self) -> &[T] {
        &self.items
    }
    /// Consumes the page and returns its items.
    pub fn into_items(self) -> Vec<T> {
        self.items
    }
    /// Returns the opaque continuation, absent on the final page.
    pub fn next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_deref()
    }
    /// Whether a continuation exists; this cannot diverge from cursor presence.
    pub fn has_more(&self) -> bool {
        self.next_cursor.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_and_defaults() {
        assert_eq!(PageLimit::new(0), Err(PageLimitError::Zero));
        for size in [101, usize::MAX] {
            assert_eq!(
                PageLimit::new(size),
                Err(PageLimitError::TooLarge {
                    requested: size,
                    maximum: 100
                })
            );
        }
        for size in 1..=100 {
            assert_eq!(PageLimit::new(size).unwrap().get(), size);
        }
        let request = PageRequest::default();
        assert_eq!(request.cursor(), None);
        assert_eq!(request.limit().get(), 50);
        let request = PageRequest::new(PageLimit::MAX).with_cursor("opaque");
        assert_eq!(request.cursor(), Some("opaque"));
        assert_eq!(request.limit().get(), 100);
    }

    #[test]
    fn response_has_more_is_derived() {
        for cursor in [None, Some("opaque".to_owned())] {
            let page = Page::new(vec![1, 2], cursor.clone());
            assert_eq!(page.items(), [1, 2]);
            assert_eq!(page.has_more(), cursor.is_some());
            assert_eq!(page.next_cursor(), cursor.as_deref());
            assert_eq!(page.into_items(), [1, 2]);
        }
    }
}
