use finance_forms::schema::{Error, compile};

const FORM:&[u8]=br#"{"version":1,"fields":[
{"key":"advance","type":"boolean","required":true,"required_when_visible":false},
{"key":"amount","type":"money","required":false,"required_when_visible":true,"visible_if":{"field":"advance","equals":true}},
{"key":"date","type":"date","required":true,"required_when_visible":false}
]}"#;

#[test]
fn dynamic_required_and_hidden_inputs() {
    let schema = compile(FORM).unwrap();
    assert_eq!(schema.version(), 1);
    assert_eq!(
        schema
            .validate_json(br#"{"advance":false,"date":"2027-01-01"}"#)
            .unwrap(),
        vec!["advance", "date"]
    );
    assert_eq!(
        schema.validate_json(br#"{"advance":true,"date":"2027-01-01"}"#),
        Err(Error::MissingRequired)
    );
    assert_eq!(
        schema.validate_json(
            br#"{"advance":false,"amount":{"amount":"10","currency":"VND"},"date":"2027-01-01"}"#
        ),
        Err(Error::HiddenValue)
    );
    assert_eq!(
        schema.validate_json(
            br#"{"advance":true,"amount":{"amount":10,"currency":"VND"},"date":"2027-01-01"}"#
        ),
        Err(Error::InvalidType)
    );
    assert_eq!(schema.validate_json(br#"{"advance":true,"amount":{"amount":"10.25","currency":"VND"},"date":"2027-01-01"}"#).unwrap(),vec!["advance","amount","date"]);
    assert_eq!(
        schema.validate_json(br#"{"advance":false,"advance":true,"date":"2027-01-01"}"#),
        Err(Error::InvalidInput)
    );
    assert_eq!(
        schema.validate_json(br#"{"advance":false,"date":"2027-02-29"}"#),
        Err(Error::InvalidType)
    );
}

#[test]
fn malformed_schema_rejected() {
    let hidden=br#"{"version":1,"fields":[{"key":"x","type":"text","required":true,"required_when_visible":false,"visible_if":{"field":"y","equals":true}}]}"#;
    assert_eq!(compile(hidden).err(), Some(Error::HiddenRequired));
    let cycle=br#"{"version":1,"fields":[{"key":"x","type":"boolean","required":false,"required_when_visible":true,"visible_if":{"field":"x","equals":true}}]}"#;
    assert_eq!(compile(cycle).err(), Some(Error::Dependency));
}

#[test]
fn label_and_layout_are_validated() {
    let invalid = br#"{"version":1,"fields":[{"key":"a","label":" ","type":"text","required":false,"required_when_visible":false}]}"#;
    assert_eq!(compile(invalid).err(), Some(Error::InvalidSchema));
    let overlap = br#"{"version":1,"fields":[
{"key":"a","label":"Amount","type":"text","required":false,"required_when_visible":false,"layout":{"row":0,"column":0,"width":8}},
{"key":"b","label":"Date","type":"date","required":false,"required_when_visible":false,"layout":{"row":0,"column":7,"width":5}}
]}"#;
    assert_eq!(compile(overlap).err(), Some(Error::InvalidSchema));
    let adjacent = br#"{"version":1,"fields":[
{"key":"a","label":"Amount","type":"text","required":false,"required_when_visible":false,"layout":{"row":0,"column":0,"width":8}},
{"key":"b","label":"Date","type":"date","required":false,"required_when_visible":false,"layout":{"row":0,"column":8,"width":4}}
]}"#;
    assert!(compile(adjacent).is_ok());
}
