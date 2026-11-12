use sticktalk_proto::*;
#[test]
fn focus_is_additive_named_wire_and_old_identify_is_unchanged() {
    let focus = ClientMsg::Focus { focused: true };
    assert_eq!(
        serde_json::to_value(&focus).unwrap(),
        serde_json::json!({"type":"focus","data":{"focused":true}})
    );
    assert_eq!(
        rmp_serde::from_slice::<ClientMsg>(&rmp_serde::to_vec_named(&focus).unwrap()).unwrap(),
        focus
    );
    let old =
        serde_json::json!({"type":"identify","data":{"version":1,"token":"native","resume":null}});
    let parsed: ClientMsg = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), old);
    assert!(
        serde_json::from_value::<ClientMsg>(serde_json::json!({"type":"focus","data":{}})).is_err()
    );
}
#[test]
fn browser_dtos_roundtrip_without_a_bearer_field() {
    let extent = HistoryExtent {
        channel_id: Id(3),
        message_count: 10,
        anchors: vec![HistoryAnchor {
            rank: 0,
            message_id: Id(5),
        }],
    };
    assert_eq!(
        serde_json::from_str::<HistoryExtent>(&serde_json::to_string(&extent).unwrap()).unwrap(),
        extent
    );
    let push = PushSubscribeRequest {
        endpoint: "https://push.test/id".into(),
        p256dh: "point".into(),
        auth: "secret".into(),
    };
    assert_eq!(
        serde_json::from_str::<PushSubscribeRequest>(&serde_json::to_string(&push).unwrap())
            .unwrap(),
        push
    );
}
