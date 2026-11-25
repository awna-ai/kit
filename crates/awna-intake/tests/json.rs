use awna_intake::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

fn triage() -> Value {
    json!({"kind":"bug","size":"small","clear":true,"summary":"The total is wrong","details":"Compare the displayed sum.\nCheck the inputs."})
}
fn ticket() -> Value {
    json!({"title":"Correct the total","spec":"Calculate the sum from its entries.","list":"To Do","blockedBy":[]})
}
fn proposal() -> Value {
    json!({"kind":"tickets","tickets":[ticket()],"reason":"The displayed sum differs from the entries."})
}
fn attachment() -> Value {
    json!({"index":0,"name":"screen.png","contentType":"image/png","role":"screenshot","size":3})
}
fn new_item() -> Value {
    json!({"source":"web-form","requesterKey":"someone@example.com","title":"The total on the summary page is wrong","body":"The entries add up to seven.","snapshot":{"state":{"page":"summary"}}})
}
fn item() -> Value {
    let mut value = new_item();
    for (key, field) in json!({"id":"IN-1","attachments":[attachment()],"triage":triage(),"proposal":proposal(),"status":"processed","events":[{"type":"status","atMs":1,"from":null,"to":"submitted","ref":null},{"type":"status","atMs":2,"from":"submitted","to":"processed","ref":null}],"createdAtMs":1,"updatedAtMs":2}).as_object().unwrap() {
        value[key] = field.clone();
    }
    value
}
fn round_trip<T: Serialize + DeserializeOwned>(value: Value) {
    let parsed: T = serde_json::from_value(value.clone()).expect("valid example");
    assert_eq!(serde_json::to_value(&parsed).unwrap(), value);
    let again: T = serde_json::from_slice(&serde_json::to_vec(&parsed).unwrap()).unwrap();
    assert_eq!(serde_json::to_value(again).unwrap(), value);
}
fn rejects<T: DeserializeOwned>(value: Value) {
    assert!(
        serde_json::from_value::<T>(value.clone()).is_err(),
        "accepted {value}"
    );
}
fn unknown<T: DeserializeOwned>(mut value: Value) {
    value["unexpected"] = json!(true);
    rejects::<T>(value);
}

#[test]
fn every_wire_type_round_trips() {
    round_trip::<ItemId>(json!("IN-1"));
    round_trip::<Source>(json!("web-form"));
    round_trip::<RequesterKey>(json!("someone@example.com"));
    round_trip::<Reference>(json!("work-19"));
    round_trip::<EventKind>(json!("review"));
    round_trip::<AttachmentName>(json!("screen.png"));
    for status in Status::ALL {
        round_trip::<Status>(json!(status.as_str()));
    }
    for value in ["file", "screenshot"] {
        round_trip::<Role>(json!(value));
    }
    for value in ["bug", "tweak", "feature", "unclear"] {
        round_trip::<TriageKind>(json!(value));
    }
    for value in ["small", "large"] {
        round_trip::<TriageSize>(json!(value));
    }
    for value in ["To Do", "Backlog"] {
        round_trip::<TicketList>(json!(value));
    }
    round_trip::<NewItem>(new_item());
    round_trip::<Snapshot>(json!({"state":[1,{"view":"summary"}]}));
    round_trip::<Attachment>(attachment());
    round_trip::<Triage>(triage());
    round_trip::<Ticket>(ticket());
    round_trip::<Blocker>(json!(0));
    round_trip::<Blocker>(json!("work-19"));
    round_trip::<Proposal>(proposal());
    for kind in ["no-action", "question"] {
        round_trip::<ProposalKind>(json!(kind));
        round_trip::<Proposal>(
            json!({"kind":kind,"tickets":[],"reason":"Ask for a reproducible example."}),
        );
    }
    round_trip::<ProposalKind>(json!("tickets"));
    round_trip::<Note>(json!({"kind":"review","body":"Checked the inputs."}));
    round_trip::<Event>(
        json!({"type":"note","atMs":2,"kind":"review","body":"Checked the inputs."}),
    );
    round_trip::<Event>(
        json!({"type":"status","atMs":2,"from":"submitted","to":"processed","ref":"work-19"}),
    );
    round_trip::<Processed>(json!({"triage":triage(),"proposal":proposal()}));
    round_trip::<Item>(item());
    round_trip::<AllowedType>(json!({"contentType":"image/png","extensions":["png"]}));
    round_trip::<Limits>(serde_json::to_value(Limits::default()).unwrap());
    round_trip::<Filter>(
        json!({"statuses":["submitted"],"source":"web-form","requesterKey":"someone@example.com","limit":2}),
    );
}

#[test]
fn filter_round_trips_with_requester_key() {
    let filter = Filter {
        requester: Some(RequesterKey::new("someone@example.com").unwrap()),
        ..Filter::default()
    };
    let value = serde_json::to_value(&filter).unwrap();
    assert_eq!(value["requesterKey"], json!("someone@example.com"));
    assert!(value.get("requester").is_none());
    assert_eq!(
        serde_json::from_value::<Filter>(value.clone()).unwrap(),
        filter
    );
    round_trip::<Filter>(value);
}

#[test]
fn every_struct_refuses_unknown_fields() {
    unknown::<NewItem>(new_item());
    unknown::<Snapshot>(json!({"state":null}));
    unknown::<Attachment>(attachment());
    unknown::<Triage>(triage());
    unknown::<Ticket>(ticket());
    unknown::<Proposal>(proposal());
    unknown::<Note>(json!({"kind":"review","body":""}));
    unknown::<Event>(json!({"type":"note","atMs":2,"kind":"review","body":""}));
    unknown::<Event>(json!({"type":"status","atMs":1,"from":null,"to":"submitted","ref":null}));
    unknown::<Processed>(json!({"triage":triage(),"proposal":proposal()}));
    unknown::<Item>(item());
    unknown::<AllowedType>(json!({"contentType":"text/plain","extensions":["txt"]}));
    unknown::<Limits>(serde_json::to_value(Limits::default()).unwrap());
    unknown::<Filter>(json!({"statuses":[],"source":null,"requesterKey":null,"limit":null}));
}

#[test]
fn absent_snapshot_is_none_and_serializes_as_null() {
    let mut value = new_item();
    value.as_object_mut().unwrap().remove("snapshot");
    let parsed: NewItem = serde_json::from_value(value).unwrap();
    assert!(parsed.snapshot.is_none());
    assert_eq!(
        serde_json::to_value(parsed).unwrap()["snapshot"],
        Value::Null
    );
}

#[test]
fn scalar_rules_apply_during_deserialization() {
    for value in ["IN-0", "IN-01", "IN-+1", "in-1", "IN-18446744073709551616"] {
        rejects::<ItemId>(json!(value));
    }
    for value in ["", "Web", "web--form", "web-", "1web", "a/b"] {
        rejects::<Source>(json!(value));
    }
    rejects::<Source>(json!("a".repeat(33)));
    rejects::<RequesterKey>(json!("a".repeat(321)));
    rejects::<Reference>(json!("a".repeat(501)));
    for value in ["status", "Review", "review--note"] {
        rejects::<EventKind>(json!(value));
    }
    for value in [
        ".",
        "..",
        ".hidden.txt",
        "a/b.txt",
        "a\\b.txt",
        "a:b.txt",
        "file",
        "file.",
        "file.a-b",
        "file.abcdefghijklmnopq",
    ] {
        rejects::<AttachmentName>(json!(value));
    }
    rejects::<Status>(json!("unknown"));
    rejects::<Role>(json!("image"));
    rejects::<TriageKind>(json!("task"));
    rejects::<TriageSize>(json!("medium"));
    rejects::<ProposalKind>(json!("none"));
    rejects::<TicketList>(json!("ToDo"));
    rejects::<Blocker>(json!(-1));
    rejects::<Blocker>(json!(4294967296_u64));
}

#[test]
fn text_fields_enforce_byte_caps_and_character_rules() {
    for (field, cap, line) in [("title", 200, true), ("body", 65536, false)] {
        let mut value = new_item();
        value[field] = json!("é".repeat(cap / 2));
        serde_json::from_value::<NewItem>(value.clone()).unwrap();
        value[field] = json!(format!("{}x", "é".repeat(cap / 2)));
        rejects::<NewItem>(value);
        for text in ["x\u{202e}y", "x\ry", "x\u{0080}y"] {
            let mut value = new_item();
            value[field] = json!(text);
            rejects::<NewItem>(value);
        }
        if line {
            for text in ["", " title", "title ", "x\ny", "x\ty"] {
                let mut value = new_item();
                value[field] = json!(text);
                rejects::<NewItem>(value);
            }
        }
    }
    for (field, cap, min) in [("summary", 280, 1), ("details", 16384, 0)] {
        let mut value = triage();
        value[field] = json!("x".repeat(cap));
        serde_json::from_value::<Triage>(value.clone()).unwrap();
        value[field] = json!("x".repeat(cap + 1));
        rejects::<Triage>(value);
        if min > 0 {
            let mut value = triage();
            value[field] = json!("");
            rejects::<Triage>(value);
        }
    }
    for (field, cap) in [("title", 200), ("spec", 32768)] {
        let mut value = ticket();
        value[field] = json!("x".repeat(cap));
        serde_json::from_value::<Ticket>(value.clone()).unwrap();
        value[field] = json!("x".repeat(cap + 1));
        rejects::<Ticket>(value);
        let mut value = ticket();
        value[field] = json!("");
        rejects::<Ticket>(value);
    }
    for length in [0, 4097] {
        let mut value = proposal();
        value["reason"] = json!("x".repeat(length));
        rejects::<Proposal>(value);
    }
    for text in ["x\u{2029}y".to_string(), "x".repeat(16385)] {
        rejects::<Note>(json!({"kind":"review","body":text}));
        rejects::<Event>(json!({"type":"note","atMs":1,"kind":"review","body":text}));
    }
}

#[test]
fn attachments_validate_metadata_without_a_store_allowlist() {
    for content_type in [
        "Image/png",
        "text",
        "/plain",
        "text/",
        "text/pl ain",
        "text/a/b",
        "text/é",
    ] {
        let mut value = attachment();
        value["contentType"] = json!(content_type);
        rejects::<Attachment>(value);
    }
    let mut value = attachment();
    value["size"] = json!(0);
    rejects::<Attachment>(value);
    let mut value = attachment();
    value["contentType"] = json!("application/custom");
    serde_json::from_value::<Attachment>(value).unwrap();
}

#[test]
fn proposals_refuse_invalid_ticket_graphs_and_shapes() {
    let mut value = proposal();
    value["tickets"] = json!([]);
    rejects::<Proposal>(value);
    let mut value = proposal();
    value["tickets"] = json!(vec![ticket(); 21]);
    rejects::<Proposal>(value);
    for kind in ["no-action", "question"] {
        let mut value = proposal();
        value["kind"] = json!(kind);
        rejects::<Proposal>(value);
    }
    for blockers in [json!([0]), json!([1]), json!(["work-19", "work-19"])] {
        let mut value = proposal();
        value["tickets"][0]["blockedBy"] = blockers;
        value["tickets"][0]["list"] = json!("Backlog");
        rejects::<Proposal>(value);
    }
    let mut value = proposal();
    value["tickets"] = json!([ticket(), ticket()]);
    value["tickets"][1]["blockedBy"] = json!([0]);
    rejects::<Proposal>(value.clone());
    value["tickets"][1]["list"] = json!("Backlog");
    serde_json::from_value::<Proposal>(value.clone()).unwrap();
    value["tickets"][1]["blockedBy"] = json!([0, 0]);
    rejects::<Proposal>(value);
    let mut value = ticket();
    value["blockedBy"] = json!(vec!["work-19"; 21]);
    rejects::<Ticket>(value);
}

#[test]
fn snapshots_enforce_compact_size_and_container_depth() {
    let mut state = json!(null);
    for _ in 0..32 {
        state = json!([state]);
    }
    serde_json::from_value::<Snapshot>(json!({"state":state.clone()})).unwrap();
    rejects::<Snapshot>(json!({"state":[state]}));
    serde_json::from_value::<Snapshot>(json!({"state":"x".repeat(1048574)})).unwrap();
    rejects::<Snapshot>(json!({"state":"x".repeat(1048575)}));
    let mut state = json!(null);
    for _ in 0..33 {
        state = json!({"child":state});
    }
    rejects::<Snapshot>(json!({"state":state}));
}

#[test]
fn item_invariants_are_enforced_on_deserialize() {
    let mut value = item();
    value["title"] = json!("x".repeat(201));
    rejects::<Item>(value);
    for change in [
        json!({"triage":null}),
        json!({"proposal":null}),
        json!({"events":[]}),
        json!({"createdAtMs":0}),
        json!({"updatedAtMs":1}),
        json!({"status":"shipped"}),
    ] {
        let mut value = item();
        for (key, field) in change.as_object().unwrap() {
            value[key] = field.clone();
        }
        rejects::<Item>(value);
    }
    let mut value = item();
    value["attachments"][0]["index"] = json!(1);
    rejects::<Item>(value);
    let mut value = item();
    let mut second = attachment();
    second["index"] = json!(1);
    value["attachments"] = json!([attachment(), second]);
    rejects::<Item>(value);
    let mut value = item();
    value["events"][0]["from"] = json!("held");
    rejects::<Item>(value);
    let mut value = item();
    value["events"][0]["to"] = json!("processed");
    rejects::<Item>(value);
    let mut value = item();
    value["events"][1]["from"] = json!("held");
    rejects::<Item>(value);
    let mut value = item();
    value["events"][1]["to"] = json!("shipped");
    value["status"] = json!("shipped");
    rejects::<Item>(value);
    let mut value = item();
    value["events"][1]["atMs"] = json!(0);
    rejects::<Item>(value);
    let mut value = item();
    value["status"] = json!("submitted");
    value["events"].as_array_mut().unwrap().pop();
    rejects::<Item>(value);
}

#[test]
fn queued_item_with_processed_history_requires_processing_results() {
    let mut value = item();
    value["status"] = json!("queued");
    value["updatedAtMs"] = json!(3);
    value["events"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"status","atMs":3,"from":"processed","to":"queued","ref":null}));
    round_trip::<Item>(value.clone());
    let mut invalid: Item = serde_json::from_value(value.clone()).unwrap();
    invalid.triage = None;
    invalid.proposal = None;
    let error = invalid.validate().unwrap_err();
    assert_eq!(error.field, "triage");
    assert_eq!(
        error.reason,
        "processing results must match processed status history"
    );
    value["triage"] = Value::Null;
    value["proposal"] = Value::Null;
    rejects::<Item>(value);
}

#[test]
fn queued_item_without_processed_history_refuses_processing_results() {
    let mut value = item();
    value["status"] = json!("queued");
    value["updatedAtMs"] = json!(3);
    value["events"][1]["to"] = json!("held");
    value["events"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"status","atMs":3,"from":"held","to":"queued","ref":null}));
    let mut valid = value.clone();
    valid["triage"] = Value::Null;
    valid["proposal"] = Value::Null;
    round_trip::<Item>(valid.clone());
    let mut invalid: Item = serde_json::from_value(valid).unwrap();
    invalid.triage = Some(serde_json::from_value(triage()).unwrap());
    invalid.proposal = Some(serde_json::from_value(proposal()).unwrap());
    let error = invalid.validate().unwrap_err();
    assert_eq!(error.field, "triage");
    assert_eq!(
        error.reason,
        "processing results must match processed status history"
    );
    rejects::<Item>(value);
}

#[test]
fn processed_parser_caps_input_and_redacts_parse_errors() {
    let too_large = " ".repeat(Processed::MAX_JSON_BYTES + 1);
    let error = Processed::from_json(&too_large).unwrap_err();
    assert_eq!(error.field, "processed");
    assert_eq!(error.reason, "model output is too large");
    let hostile = "PRIVATE_INPUT_DO_NOT_ECHO";
    let error = Processed::from_json(hostile).unwrap_err();
    assert_eq!(error.field, "processed");
    assert_eq!(error.reason, "not a valid processed object");
    assert!(!error.to_string().contains(hostile));
    let valid = json!({"triage":triage(),"proposal":proposal()}).to_string();
    Processed::from_json(&valid).unwrap();
    let mut value = json!({"triage":triage(),"proposal":proposal()});
    value["proposal"]["tickets"] = json!([]);
    assert!(Processed::from_json(&value.to_string()).is_err());
}

#[test]
fn contract_examples_parse_as_the_promised_types() {
    let contract =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/CONTRACT.md")).unwrap();
    let example = |name: &str| {
        contract
            .split(&format!("```json {name}\n"))
            .nth(1)
            .expect("example fence")
            .split("```")
            .next()
            .unwrap()
    };
    let item: Item = serde_json::from_str(example("item")).expect("valid contract item");
    item.validate().unwrap();
    assert_eq!(item.status, Status::Processed);
    let processed: Processed =
        serde_json::from_str(example("processed")).expect("valid contract output");
    processed.validate().unwrap();
}

#[test]
fn every_text_boundary_rejects_forbidden_characters() {
    for character in [
        '\u{001f}', '\u{007f}', '\u{0080}', '\u{009f}', '\u{2028}', '\u{2029}', '\u{202a}',
        '\u{202e}', '\u{2066}', '\u{2069}', '\u{061c}', '\u{200e}', '\u{200f}',
    ] {
        let text = format!("a{character}b");
        rejects::<RequesterKey>(json!(text));
        rejects::<Reference>(json!(text));
        let mut value = triage();
        value["summary"] = json!(text);
        rejects::<Triage>(value);
        let mut value = triage();
        value["details"] = json!(text);
        rejects::<Triage>(value);
        let mut value = ticket();
        value["title"] = json!(text);
        rejects::<Ticket>(value);
        let mut value = ticket();
        value["spec"] = json!(text);
        rejects::<Ticket>(value);
        let mut value = proposal();
        value["reason"] = json!(text);
        rejects::<Proposal>(value);
        rejects::<Note>(json!({"kind":"review","body":text}));
    }
    for text in ["", " leading", "trailing ", "a\nb", "a\tb", "a\rb"] {
        rejects::<RequesterKey>(json!(text));
        rejects::<Reference>(json!(text));
        let mut value = triage();
        value["summary"] = json!(text);
        rejects::<Triage>(value);
        let mut value = ticket();
        value["title"] = json!(text);
        rejects::<Ticket>(value);
    }
    for text in ["a\nb", "a\tb"] {
        let mut value = new_item();
        value["body"] = json!(text);
        serde_json::from_value::<NewItem>(value).unwrap();
        let mut value = triage();
        value["details"] = json!(text);
        serde_json::from_value::<Triage>(value).unwrap();
        let mut value = ticket();
        value["spec"] = json!(text);
        serde_json::from_value::<Ticket>(value).unwrap();
        let mut value = proposal();
        value["reason"] = json!(text);
        serde_json::from_value::<Proposal>(value).unwrap();
        serde_json::from_value::<Note>(json!({"kind":"review","body":text})).unwrap();
    }
    round_trip::<RequesterKey>(json!("é".repeat(160)));
    round_trip::<Reference>(json!("é".repeat(250)));
    round_trip::<Source>(json!("a".repeat(32)));
    round_trip::<AttachmentName>(json!(format!("{}.txt", "a".repeat(251))));
    rejects::<AttachmentName>(json!(format!("{}.txt", "a".repeat(252))));
    let mut value = attachment();
    value["contentType"] = json!(format!("a/{}", "b".repeat(125)));
    serde_json::from_value::<Attachment>(value).unwrap();
    let mut value = attachment();
    value["contentType"] = json!(format!("a/{}", "b".repeat(126)));
    rejects::<Attachment>(value);
    let mut value = ticket();
    value["blockedBy"] = json!(
        (0..20)
            .map(|index| format!("work-{index}"))
            .collect::<Vec<_>>()
    );
    serde_json::from_value::<Ticket>(value.clone()).unwrap();
    value["blockedBy"]
        .as_array_mut()
        .unwrap()
        .push(json!("work-20"));
    rejects::<Ticket>(value);
}

#[test]
fn nested_errors_preserve_json_field_paths_without_echoing_text() {
    let mut value = proposal();
    value["tickets"] = json!([ticket(), ticket(), ticket()]);
    value["tickets"][2]["title"] = json!("bad\u{202e}title");
    let built: Proposal = serde_json::from_value(proposal()).unwrap();
    let mut invalid = built;
    invalid
        .tickets
        .push(serde_json::from_value(ticket()).unwrap());
    invalid
        .tickets
        .push(serde_json::from_value(ticket()).unwrap());
    invalid.tickets[2].title = "bad\u{202e}title".into();
    let output = Processed {
        triage: serde_json::from_value(triage()).unwrap(),
        proposal: invalid,
    };
    let error = output.validate().unwrap_err();
    assert_eq!(error.field, "proposal.tickets[2].title");
    assert!(!error.to_string().contains("bad"));
    rejects::<Proposal>(value);
}

#[test]
fn processed_parser_accepts_exact_cap_and_keeps_default_recursion_limit() {
    let valid = json!({"triage":triage(),"proposal":proposal()}).to_string();
    let padded = format!(
        "{valid}{}",
        " ".repeat(Processed::MAX_JSON_BYTES - valid.len())
    );
    assert_eq!(padded.len(), Processed::MAX_JSON_BYTES);
    Processed::from_json(&padded).unwrap();
    let mut deep_output = json!({"triage":triage(),"proposal":proposal()});
    deep_output["proposal"]["tickets"][0]["blockedBy"] = json!(["nested-value"]);
    // Untagged blockers buffer their input, so this known field reaches the
    // parser's nesting guard before blocker shape validation can reject it.
    let nested = deep_output.to_string().replace(
        "\"nested-value\"",
        &format!("{}0{}", "[".repeat(130), "]".repeat(130)),
    );
    let direct_error = serde_json::from_str::<Processed>(&nested).unwrap_err();
    assert!(
        direct_error
            .to_string()
            .contains("recursion limit exceeded")
    );
    let error = Processed::from_json(&nested).unwrap_err();
    assert_eq!(error.reason, "not a valid processed object");
    let hostile = "untrusted\rvalue";
    let mut invalid = json!({"triage":triage(),"proposal":proposal()});
    invalid["triage"]["summary"] = json!(hostile);
    let error = Processed::from_json(&invalid.to_string()).unwrap_err();
    assert_eq!(error.field, "processed");
    assert_eq!(error.reason, "not a valid processed object");
    assert!(!error.to_string().contains("untrusted"));
}

#[test]
fn configurable_limits_refuse_invalid_allowlist_values() {
    for content_type in ["Text/plain", "text/plain\n", "text"] {
        rejects::<AllowedType>(json!({"contentType":content_type,"extensions":["txt"]}));
    }
    for extension in ["", "TXT", ".txt", "a-b", "abcdefghijklmnopq", "é"] {
        let allowed = json!({"contentType":"text/plain","extensions":[extension]});
        rejects::<AllowedType>(allowed.clone());
        rejects::<Limits>(
            json!({"maxFiles":3,"maxFileBytes":1024,"maxItemBytes":2048,"allowed":[allowed]}),
        );
    }
}
