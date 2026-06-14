#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppStateDetail {
    Full,
    Compact,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct AppStateElementOptions {
    pub(crate) element_offset: usize,
    pub(crate) element_limit: Option<usize>,
    pub(crate) element_query: Option<String>,
}

impl AppStateElementOptions {
    #[must_use]
    pub(crate) fn constrains_elements(&self) -> bool {
        self.element_offset > 0 || self.element_limit.is_some() || self.element_query.is_some()
    }
}

pub(crate) const APP_STATE_DEFAULT_ELEMENT_LIMIT: usize = 200;
pub(crate) const APP_STATE_MAX_ELEMENT_LIMIT: usize = 5_000;
pub(crate) const APP_STATE_MAX_ELEMENT_QUERY_CHARS: usize = 256;
