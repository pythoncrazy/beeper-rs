//! Scratch, not committed: the chats beeper-rs imports from a Desktop data folder.
#[test]
#[ignore]
fn dump_import() {
    let dir = std::env::var("REAL_DIR").unwrap();
    let data = beeper_core::import::read_beeper_data(std::path::Path::new(&dir)).unwrap();
    let rows: Vec<serde_json::Value> = data
        .chats
        .iter()
        .map(|c| {
            serde_json::json!({ "id": c.id, "account": c.account_id, "title": c.title, "type": c.chat_type,
                "flags": c.flags, "archived": c.archived_in_list(), "hidden": c.is_hidden_from_inbox, "merged": c.merged_into_chat_id, "isArchived": c.is_archived, "lowp": c.is_low_priority })
        })
        .collect();
    std::fs::write(format!("{dir}/rs-import.json"), serde_json::to_string(&rows).unwrap()).unwrap();
}
