#[derive(Debug, PartialEq, Eq)]
pub struct Query<T> {
    result: QueryResult<T>,
    activity: QueryActivity,
}

#[derive(Debug, PartialEq, Eq)]
pub enum QueryResult<T> {
    Pending,
    Success(T),
    Error { data: Option<T>, error: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryActivity {
    Idle,
    Fetching,
}

impl<T> Default for Query<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Query<T> {
    pub fn new() -> Self {
        Self {
            result: QueryResult::Pending,
            activity: QueryActivity::Idle,
        }
    }

    pub fn result(&self) -> &QueryResult<T> {
        &self.result
    }

    pub fn activity(&self) -> QueryActivity {
        self.activity
    }

    pub fn data(&self) -> Option<&T> {
        match &self.result {
            QueryResult::Pending => None,
            QueryResult::Success(data) => Some(data),
            QueryResult::Error { data, .. } => data.as_ref(),
        }
    }

    pub fn error(&self) -> Option<&str> {
        match &self.result {
            QueryResult::Error { error, .. } => Some(error),
            _ => None,
        }
    }

    pub fn is_pending(&self) -> bool {
        matches!(self.result, QueryResult::Pending)
    }

    pub fn is_fetching(&self) -> bool {
        self.activity == QueryActivity::Fetching
    }

    pub fn is_loading(&self) -> bool {
        self.is_pending() && self.is_fetching()
    }

    pub fn is_refreshing(&self) -> bool {
        self.data().is_some() && self.is_fetching()
    }

    pub fn start(&mut self) {
        self.activity = QueryActivity::Fetching;
    }

    /// The caller owns request identity and must reject stale completions.
    pub fn complete(&mut self, result: Result<T, String>) {
        self.result = match result {
            Ok(data) => QueryResult::Success(data),
            Err(error) => {
                let data = match std::mem::replace(&mut self.result, QueryResult::Pending) {
                    QueryResult::Success(data) => Some(data),
                    QueryResult::Error { data, .. } => data,
                    QueryResult::Pending => None,
                };
                QueryResult::Error { data, error }
            }
        };
        self.activity = QueryActivity::Idle;
    }

    /// Changes query state only; the caller must stop or invalidate the request.
    pub fn cancel(&mut self) {
        self.activity = QueryActivity::Idle;
    }
}

#[cfg(test)]
mod tests;
