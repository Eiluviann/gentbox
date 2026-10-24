//! A message delivered without a ping because of a mute says so in the ingest
//! response (the `muted` warning), whatever its type and whether or not the
//! sender asks; urgent messages break through and get no warning.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use gentbox_core::classify::RuleSet;
use gentbox_core::response::{IngestResponse, IngestWarning, WarningCode};
use gentbox_core::topic::TopicCreate;
use gentbox_hub::IngestStatus;
use serde_json::json;
use support::{TestHub, blocking_prompt};

fn muted_warning(response: &IngestResponse) -> Option<&IngestWarning> {
    response
        .warnings
        .iter()
        .find(|warning| warning.code == WarningCode::Muted)
}

async fn muted_topic(hub: &TestHub) -> gentbox_core::topic::TopicRecord {
    let request: TopicCreate =
        serde_json::from_value(json!({ "key": "release-1.4", "title": "Release 1.4" })).unwrap();
    let topic = hub
        .handle
        .create_topic("claude-code".to_owned(), request)
        .await
        .unwrap()
        .into_inner();
    hub.handle.mute_topic(topic.id.clone(), true).await.unwrap();
    topic
}

#[tokio::test]
async fn an_ask_in_a_muted_topic_reports_the_mute_and_names_the_topic() {
    let hub = TestHub::start();
    muted_topic(&hub).await;
    let mut prompt = blocking_prompt();
    prompt["topic"] = json!({ "key": "release-1.4" });
    let response = hub.create(prompt).await;
    assert!(!response.should_wait);
    let warning = muted_warning(&response).expect("a muted warning");
    assert_eq!(
        warning.message,
        "Delivered to the inbox without a ping: topic 'Release 1.4' is muted"
    );
    hub.stop().await;
}

#[tokio::test]
async fn a_notification_in_a_muted_topic_reports_the_mute_too() {
    let hub = TestHub::start();
    muted_topic(&hub).await;
    let response = hub
        .create(json!({
            "type": "task_completed", "title": "Release notes are drafted",
            "topic": { "key": "release-1.4" }
        }))
        .await;
    assert!(muted_warning(&response).is_some(), "{response:?}");
    hub.stop().await;
}

#[tokio::test]
async fn a_message_in_a_muted_thread_reports_the_mute_and_names_the_thread() {
    let hub = TestHub::start();
    let first = hub.create(blocking_prompt()).await;
    hub.handle
        .mute_thread(first.thread_id.clone(), true)
        .await
        .unwrap();
    let thread = hub.handle.thread(first.thread_id.clone()).await.unwrap();

    let second = hub
        .create(json!({ "type": "generic", "title": "Started phase 3" }))
        .await;
    let warning = muted_warning(&second).expect("a muted warning");
    assert_eq!(
        warning.message,
        format!(
            "Delivered to the inbox without a ping: thread '{}' is muted",
            thread.title
        )
    );
    hub.stop().await;
}

#[tokio::test]
async fn a_message_muted_by_a_rule_reports_the_rule() {
    let rules = RuleSet::parse(
		"[[rule]]\nname = \"Mute prompts\"\nmatch = { event = \"claude-code.permission_request\" }\nmute = true\n",
	)
	.unwrap();
    let hub = TestHub::start_with(|config| config.rules = rules);
    let mut prompt = blocking_prompt();
    prompt["event"] = json!("claude-code.permission_request");
    let response = hub.create(prompt).await;
    assert!(!response.should_wait);
    let warning = muted_warning(&response).expect("a muted warning");
    assert_eq!(
        warning.message,
        "Delivered to the inbox without a ping: rule 'Mute prompts' mutes it"
    );
    hub.stop().await;
}

#[tokio::test]
async fn an_urgent_message_that_breaks_through_a_mute_gets_no_warning() {
    let hub = TestHub::start();
    muted_topic(&hub).await;
    let mut prompt = blocking_prompt();
    prompt["topic"] = json!({ "key": "release-1.4" });
    prompt["urgent"] = json!(true);
    let response = hub.create(prompt).await;
    assert!(response.should_wait);
    assert!(muted_warning(&response).is_none(), "{response:?}");
    hub.stop().await;
}

#[tokio::test]
async fn an_unmuted_message_gets_no_muted_warning() {
    let hub = TestHub::start();
    let response = hub.create(blocking_prompt()).await;
    assert!(muted_warning(&response).is_none());
    hub.stop().await;
}

#[tokio::test]
async fn an_idempotent_retry_of_a_muted_ask_reports_the_mute_again() {
    let hub = TestHub::start();
    muted_topic(&hub).await;
    let mut prompt = blocking_prompt();
    prompt["topic"] = json!({ "key": "release-1.4" });
    prompt["idempotencyKey"] = json!("retry-1");
    let first = hub.create(prompt.clone()).await;
    let retry = hub.post(prompt).await;
    assert_eq!(retry.status, IngestStatus::Existing);
    assert_eq!(retry.response.id, first.id);
    assert!(!retry.response.should_wait);
    let warning = muted_warning(&retry.response).expect("a muted warning");
    assert_eq!(
        warning.message,
        "Delivered to the inbox without a ping: topic 'Release 1.4' is muted"
    );
    hub.stop().await;
}

#[tokio::test]
async fn a_retry_of_an_unmuted_or_closed_message_gets_no_muted_warning() {
    let hub = TestHub::start();
    let mut prompt = blocking_prompt();
    prompt["idempotencyKey"] = json!("retry-2");
    let first = hub.create(prompt.clone()).await;
    assert!(muted_warning(&hub.post(prompt).await.response).is_none());

    // Muted later and then closed: nothing is held back any more, so nothing is reported.
    hub.handle
        .mute_thread(first.thread_id.clone(), true)
        .await
        .unwrap();
    hub.press(&first.id, "allow").await.unwrap();
    let mut again = blocking_prompt();
    again["idempotencyKey"] = json!("retry-2");
    assert!(muted_warning(&hub.post(again).await.response).is_none());
    hub.stop().await;
}

#[tokio::test]
async fn a_rule_muted_ask_that_folds_into_a_live_ask_does_not_report_a_mute() {
    // The response names the live ask, not the muted one. A `muted` warning
    // would lead the sender to rescind a message someone is waiting on.
    let rules = RuleSet::parse(
        "[[rule]]\nname = \"Mute prompts\"\nmatch = { event = \"claude-code.permission_request\" }\nmute = true\n",
    )
    .unwrap();
    let hub = TestHub::start_with(|config| config.rules = rules);
    let live = hub.create(blocking_prompt()).await;
    assert!(live.should_wait);

    let mut muted = blocking_prompt();
    muted["event"] = json!("claude-code.permission_request");
    muted["title"] = json!("Allow Bash to run every test?");
    let folded = hub.post(muted).await;
    assert_eq!(folded.status, IngestStatus::Existing);
    assert_eq!(folded.response.id, live.id);
    assert!(!folded.response.should_wait);
    assert!(
        muted_warning(&folded.response).is_none(),
        "{:?}",
        folded.response
    );
    hub.stop().await;
}
