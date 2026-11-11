use super::*;
use comet_asset_data::game_data::MAX_DATA_RECORDS;
use serde_json::json;

#[test]
fn csv_framing_and_typed_cells_match_json_and_reject_invalid_or_excessive_input() -> Result<()> {
    let id = AssetId::new();
    let schema = json!({"fields":{
        "title":{"type":"string"}, "hp":{"type":"integer","min":1},
        "ratio":{"type":"number"}, "ready":{"type":"bool"},
        "extra":{"type":"json"}, "asset":{"type":"asset","asset_type":"data"}
    }});
    let DataValue::Object(mut spec) = serde_json::from_value(schema.clone())? else {
        unreachable!()
    };
    let fields = fields(&mut spec)?;
    let text = format!(
        "\u{feff}id,title,hp,ratio,ready,extra,asset\r\nhero,\"英雄, \"\"甲\"\"\r\n继续\",7,1.25,true,\"{{\"\"a\"\":[false,2]}}\",{id}\r\nother,,3,2,false,[],{id}"
    );
    let parsed = normalize(fields.clone(), csv::records(text.as_bytes(), &fields)?)?;
    let mut json = schema;
    json["rows"] = json!([
        {"id":"hero","title":"英雄, \"甲\"\r\n继续","hp":7,"ratio":1.25,"ready":true,"extra":{"a":[false,2]},"asset":id},
        {"id":"other","title":"","hp":3,"ratio":2,"ready":false,"extra":[],"asset":id}
    ]);
    assert_eq!(parsed, import(&serde_json::to_vec(&json)?)?);
    let fields = [("text".into(), DataField::String)].into();
    for bytes in [
        b"id,text\nhero,\"bad".as_slice(),
        b"id,text\nhero,b\"ad",
        b"id,text\nhero,\"ok\"tail",
        b"id,text,text\nhero,a,b",
        b"id,other\nhero,a",
        b"id\nhero",
        b"id,text\nhero",
        b"id,text\nhero,a,b",
        b"id,text\nhero,a\nhero,b",
        b"id,text\n,a",
        b"id,text\nhero,\xff",
        b"",
    ] {
        assert!(
            csv::records(bytes, &fields)
                .and_then(|rows| normalize(fields.clone(), rows))
                .is_err(),
            "{bytes:?}"
        );
    }
    assert!(csv::records(&vec![b'x'; MAX_DATA_BYTES + 1], &fields).is_err());
    let bytes = format!(
        "id,text\n{}",
        (0..=MAX_DATA_RECORDS)
            .map(|n| format!("row{n},ok\n"))
            .collect::<String>()
    );
    assert!(csv::records(bytes.as_bytes(), &fields).is_err());
    assert!(csv::records(format!("{}\n", "x,".repeat(34)).as_bytes(), &fields).is_err());
    let fields = [("value".into(), DataField::Json)].into();
    for cell in ["null", "{broken}", "9007199254740992", "1e999"] {
        let bytes = format!("id,value\nhero,{cell}");
        assert!(
            csv::records(bytes.as_bytes(), &fields)
                .and_then(|rows| normalize(fields.clone(), rows))
                .is_err()
        );
    }
    let rows = csv::records(b"id\nhero\n", &BTreeMap::new())?;
    assert_eq!(normalize(BTreeMap::new(), rows)?.rows.len(), 1);
    Ok(())
}
