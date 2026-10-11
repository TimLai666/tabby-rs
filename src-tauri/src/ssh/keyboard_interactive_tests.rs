use super::{
    authenticate_keyboard_interactive_with,
    engine::{
        KeyboardInteractivePrompt, KeyboardInteractivePromptItem, KeyboardInteractiveResponse,
        PrivateKeyMaterial, SshAuthContext,
    },
    SshError,
};
use secrecy::SecretString;
use std::{
    collections::VecDeque,
    future::ready,
    sync::{Arc, Mutex},
};

struct Context {
    rounds: VecDeque<Result<KeyboardInteractiveResponse, SshError>>,
    sent: Vec<Vec<String>>,
    starts: usize,
    respond_error: Option<SshError>,
}

impl Context {
    fn new(rounds: impl IntoIterator<Item = KeyboardInteractiveResponse>) -> Self {
        Self {
            rounds: rounds.into_iter().map(Ok).collect(),
            sent: vec![],
            starts: 0,
            respond_error: None,
        }
    }
    fn next(&mut self) -> Result<KeyboardInteractiveResponse, SshError> {
        self.rounds
            .pop_front()
            .unwrap_or(Ok(KeyboardInteractiveResponse::Failure))
    }
}

#[async_trait::async_trait]
impl SshAuthContext for Context {
    async fn authenticate_none(&mut self, _: &str) -> Result<bool, SshError> {
        unreachable!()
    }
    async fn authenticate_password(&mut self, _: &str, _: &SecretString) -> Result<bool, SshError> {
        unreachable!()
    }
    async fn authenticate_private_key(
        &mut self,
        _: &str,
        _: PrivateKeyMaterial,
    ) -> Result<bool, SshError> {
        unreachable!()
    }
    async fn authenticate_agent(&mut self, _: &str, _: Option<&str>) -> Result<bool, SshError> {
        unreachable!()
    }
    async fn authenticate_keyboard_interactive_start(
        &mut self,
        _: &str,
    ) -> Result<KeyboardInteractiveResponse, SshError> {
        self.starts += 1;
        self.next()
    }
    async fn authenticate_keyboard_interactive_respond(
        &mut self,
        answers: Vec<String>,
    ) -> Result<KeyboardInteractiveResponse, SshError> {
        self.sent.push(answers);
        if let Some(error) = self.respond_error.take() {
            return Err(error);
        }
        self.next()
    }
}

fn challenge(prompts: usize) -> KeyboardInteractiveResponse {
    KeyboardInteractiveResponse::Prompt(KeyboardInteractivePrompt {
        name: "fixture".into(),
        instructions: "fixture instructions".into(),
        prompts: (0..prompts)
            .map(|i| KeyboardInteractivePromptItem {
                text: format!("answer {i}"),
                echo: false,
            })
            .collect(),
    })
}

#[tokio::test]
async fn empty_reply_to_nonempty_challenge_is_candidate_cancellation() {
    let mut context = Context::new([challenge(1), KeyboardInteractiveResponse::Success]);
    let ui_calls = Arc::new(Mutex::new(0));
    let calls = ui_calls.clone();
    let result = authenticate_keyboard_interactive_with(&mut context, "alice", move |_| {
        *calls.lock().unwrap() += 1;
        ready(Ok(vec![]))
    })
    .await;
    assert!(matches!(result, Ok(false)));
    assert_eq!(*ui_calls.lock().unwrap(), 1);
    assert!(
        context.sent.is_empty(),
        "a dismissed round must not be sent"
    );
}

#[tokio::test]
async fn dismissal_on_second_round_keeps_first_answer_and_sends_no_second_answer() {
    let mut context = Context::new([
        challenge(1),
        challenge(1),
        KeyboardInteractiveResponse::Success,
    ]);
    let mut call = 0;
    let result = authenticate_keyboard_interactive_with(&mut context, "alice", |_| {
        call += 1;
        ready(Ok(if call == 1 {
            vec!["first".into()]
        } else {
            vec![]
        }))
    })
    .await;
    assert!(matches!(result, Ok(false)));
    assert_eq!(context.sent, [vec!["first".to_owned()]]);
}

#[tokio::test]
async fn empty_challenges_automatically_send_empty_answers_without_ui() {
    let mut context = Context::new([
        challenge(0),
        challenge(0),
        KeyboardInteractiveResponse::Success,
    ]);
    let mut calls = 0;
    let result = authenticate_keyboard_interactive_with(&mut context, "alice", |_| {
        calls += 1;
        ready(Ok(vec![]))
    })
    .await;
    assert!(matches!(result, Ok(true)));
    assert_eq!(calls, 0);
    assert_eq!(context.sent, [Vec::<String>::new(), Vec::<String>::new()]);
}

#[tokio::test]
async fn empty_challenge_then_nonempty_dismissal_only_sends_automatic_reply() {
    let mut context = Context::new([
        challenge(0),
        challenge(1),
        KeyboardInteractiveResponse::Success,
    ]);
    let result =
        authenticate_keyboard_interactive_with(&mut context, "alice", |_| ready(Ok(vec![]))).await;
    assert!(matches!(result, Ok(false)));
    assert_eq!(context.sent, [Vec::<String>::new()]);
}

#[tokio::test]
async fn one_empty_string_is_a_submitted_blank_answer() {
    let mut context = Context::new([challenge(1), KeyboardInteractiveResponse::Success]);
    let result = authenticate_keyboard_interactive_with(&mut context, "alice", |_| {
        ready(Ok(vec![String::new()]))
    })
    .await;
    assert!(matches!(result, Ok(true)));
    assert_eq!(context.sent, [vec![String::new()]]);
}

#[tokio::test]
async fn multi_field_answers_preserve_server_success_and_failure() {
    for (terminal, expected) in [
        (KeyboardInteractiveResponse::Success, true),
        (KeyboardInteractiveResponse::Failure, false),
    ] {
        let mut context = Context::new([challenge(2), terminal]);
        let result = authenticate_keyboard_interactive_with(&mut context, "alice", |_| {
            ready(Ok(vec!["password".into(), "otp".into()]))
        })
        .await;
        assert!(matches!(result, Ok(value) if value == expected));
        assert_eq!(
            context.sent,
            [vec!["password".to_owned(), "otp".to_owned()]]
        );
    }
}

#[tokio::test]
async fn prompt_errors_propagate_without_responding() {
    for error in [SshError::Closed, SshError::Timeout, SshError::Internal] {
        let mut context = Context::new([challenge(1)]);
        let expected = error.code();
        let result = authenticate_keyboard_interactive_with(&mut context, "alice", move |_| {
            let error = match expected {
                "closed" => SshError::Closed,
                "timeout" => SshError::Timeout,
                _ => SshError::Internal,
            };
            ready(Err(error))
        })
        .await;
        assert!(matches!(result, Err(actual) if actual.code() == expected));
        assert!(context.sent.is_empty());
    }
}

#[tokio::test]
async fn start_and_respond_transport_errors_propagate() {
    let mut start_error = Context::new([]);
    start_error.rounds.push_front(Err(SshError::Connection));
    let result = authenticate_keyboard_interactive_with(&mut start_error, "alice", |_| {
        ready(Ok(vec!["x".into()]))
    })
    .await;
    assert!(matches!(result, Err(SshError::Connection)));

    let mut respond_error = Context::new([]);
    respond_error.rounds.push_back(Ok(challenge(1)));
    respond_error.respond_error = Some(SshError::Connection);
    let result = authenticate_keyboard_interactive_with(&mut respond_error, "alice", |_| {
        ready(Ok(vec!["x".into()]))
    })
    .await;
    assert!(matches!(result, Err(SshError::Connection)));
    assert_eq!(respond_error.sent, [vec!["x".to_owned()]]);
}
