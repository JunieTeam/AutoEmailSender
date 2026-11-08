//! An in-memory feedback endpoint following the official server's rules (the server repository's
//! docs/feedback.md): it takes each feedback in, then answers as told.
use cloud_api::feedback::Feedback;
use serde_json::json;

use crate::http::{Reply, error, ok};

#[derive(Default)]
pub struct State {
    /// Every feedback that reached the server, in order.
    pub received: Vec<Feedback>,
    /// The status and error code answered, such as when passing it on failed after it arrived;
    /// success when none.
    pub answer: Option<(u16, &'static str)>,
}

pub type Server = crate::http::Server<State>;

impl Server {
    pub async fn start() -> std::io::Result<Self> {
        crate::http::Server::serve(State::default(), respond).await
    }
}

#[expect(
    clippy::unwrap_used,
    reason = "test fixture request failures must panic at their source"
)]
fn respond(state: &mut State, method: &str, path: &str, body: Vec<u8>) -> Reply {
    if (method, path) != ("POST", cloud_api::path::FEEDBACK) {
        return error(404, "not_found");
    }
    state.received.push(serde_json::from_slice(&body).unwrap());
    match state.answer {
        Some((status, code)) => error(status, code),
        None => ok(json!({})),
    }
}
