use super::*;

#[test]
fn pending_is_not_loading_until_started() {
    let mut query = Query::<String>::new();
    assert_eq!(query, Query::default());
    assert_eq!(query.result(), &QueryResult::Pending);
    assert_eq!(query.activity(), QueryActivity::Idle);
    assert!(query.is_pending());
    assert!(!query.is_loading());
    assert!(!query.is_refreshing());
    assert_eq!(query.data(), None);
    assert_eq!(query.error(), None);

    query.start();
    assert!(query.is_pending());
    assert!(query.is_loading());
    assert!(query.is_fetching());
    assert!(!query.is_refreshing());

    query.complete(Ok("first".into()));
    assert_eq!(query.result(), &QueryResult::Success("first".into()));
    assert_eq!(query.activity(), QueryActivity::Idle);
    assert!(!query.is_pending());
    assert!(!query.is_loading());
    assert_eq!(query.data().map(String::as_str), Some("first"));
}

#[test]
fn mutable_data_preserves_activity_and_errors() {
    let mut query = Query::<i32>::new();
    assert!(query.data_mut().is_none());
    query.start();
    query.complete(Err("initial error".into()));
    assert!(query.data_mut().is_none());
    query.complete(Ok(1));
    *query.data_mut().unwrap() = 2;
    assert_eq!(query.data(), Some(&2));
    assert!(!query.is_fetching());

    query.start();
    query.complete(Err("refresh error".into()));
    query.start();
    *query.data_mut().unwrap() = 3;
    assert_eq!(query.data(), Some(&3));
    assert_eq!(query.error(), Some("refresh error"));
    assert!(query.is_fetching());
}

#[test]
fn refresh_retains_data_until_replaced() {
    let mut query = Query::new();
    query.start();
    query.complete(Ok(1));
    query.start();
    query.start();
    assert!(query.is_refreshing());
    assert!(!query.is_loading());
    assert_eq!(query.data(), Some(&1));

    query.complete(Ok(2));
    assert_eq!(query.data(), Some(&2));
    assert!(!query.is_fetching());
}

#[test]
fn failed_refresh_keeps_data_and_error_during_retry() {
    let mut query = Query::new();
    query.start();
    query.complete(Ok(1));
    query.start();
    query.complete(Err("first error".into()));
    assert_eq!(query.data(), Some(&1));
    assert_eq!(query.error(), Some("first error"));
    assert!(!query.is_fetching());

    query.start();
    assert!(query.is_refreshing());
    assert_eq!(query.error(), Some("first error"));
    query.complete(Err("second error".into()));
    assert_eq!(query.data(), Some(&1));
    assert_eq!(query.error(), Some("second error"));

    query.start();
    query.complete(Ok(2));
    assert_eq!(query.data(), Some(&2));
    assert_eq!(query.error(), None);
    assert!(!query.is_fetching());
}

#[test]
fn initial_failure_can_retry_without_data() {
    let mut query = Query::<String>::new();
    query.start();
    query.complete(Err("unavailable".into()));
    assert_eq!(query.data(), None);
    assert_eq!(query.error(), Some("unavailable"));
    assert!(!query.is_pending());
    assert!(!query.is_fetching());

    query.start();
    assert!(query.is_fetching());
    assert!(!query.is_loading());
    assert!(!query.is_refreshing());
    assert_eq!(query.error(), Some("unavailable"));
    query.complete(Ok("recovered".into()));
    assert_eq!(query.data().map(String::as_str), Some("recovered"));
    assert_eq!(query.error(), None);
}

#[test]
fn cancellation_preserves_every_previous_result() {
    let results: [fn() -> QueryResult<i32>; 4] = [
        || QueryResult::Pending,
        || QueryResult::Success(1),
        || QueryResult::Error {
            data: None,
            error: "initial error".into(),
        },
        || QueryResult::Error {
            data: Some(1),
            error: "refresh error".into(),
        },
    ];
    for result in results {
        let mut query = Query {
            result: result(),
            activity: QueryActivity::Idle,
        };
        query.start();
        query.cancel();
        query.cancel();
        assert_eq!(query.activity(), QueryActivity::Idle);
        assert_eq!(query.result(), &result());
    }
}
