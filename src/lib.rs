use std::{collections::HashMap, fmt::{Debug, Display}};

use serde_json::json;

use crate::{block::Block, comment::Comment, editor::Editor, field_types::VSFieldType, unique_id::UniqueIdService};

pub mod base93;
pub mod deflate;
pub mod decoder;
pub mod encoder;
pub mod transpile;
pub mod field_types;
pub mod editor;
pub mod block;
pub mod comment;
pub mod hex;
pub mod macros;
pub mod unique_id;

const LATEST_VS_VERSION: u8 = 4;

/// The embedded data-driven block catalog (Phase 3).
pub mod vs_blocks {
    /// Raw JSON of `src/vs_blocks.json`.
    pub const VS_BLOCKS_JSON: &str = include_str!("vs_blocks.json");

    /// Parses the catalog into a serde_json value.
    pub fn catalog() -> serde_json::Value {
        serde_json::from_str(VS_BLOCKS_JSON).expect("vs_blocks.json is valid JSON")
    }
}

const U_001A: &str = "\u{001A}";
const U_001B: &str = "\u{001B}";

trait VSObjectType {
    fn to_json(&self) -> serde_json::Value;
    fn from_json(&mut self, json: serde_json::Value) -> Result<(), &'static str>;
    fn get_type(&self) -> &'static str;
}

#[derive(Debug)]
pub struct VisualSource {
    /// Depicts the version of the visual source
    pub version: u8,

    /// Editor instance. It represents the camera in the script editor
    pub editor: Editor,

    /// HashMap where the key is the block name, and the value is the actual block instance
    pub blocks: HashMap<String, Block>,

    /// Vector of comments (not the Comment block)
    pub comments: Vec<Comment>,

    pub unique_id_service: UniqueIdService
}

impl VisualSource {
    /// Decodes a VisualSource payload string into a DecodedReport
    pub fn decode(blob_text: &str) -> Result<decoder::DecodedReport, decoder::DecodeError> {
        decoder::decode_blob(blob_text)
    }

    pub fn to_json(&self) -> serde_json::Value {
        let editor = self.editor.to_json();
        let blocks = self.blocks.iter().map(|(block_name, block)| (block_name, block.to_json())).collect::<HashMap<_, _>>();
        let comments = self.comments.iter().map(|comment| comment.to_json()).collect::<Vec<serde_json::Value>>();

        json!({
            "Editor": editor,
            "Blocks": blocks,
            "Comments": comments
        })
    }

    pub fn from_json(mut json: serde_json::Value) -> Result<Self, &'static str> {
        let editor = Editor::from_json(json.get_mut("Editor").ok_or("'Editor' does not exist in Visual Source json")?.take())?;
        let blocks = json.get_mut("Blocks").ok_or("'Blocks' does not exist in Visual Source json")?.take()
                                                .as_array_mut().ok_or("'Blocks' in Visual Source json is not an array")?
                                                .iter_mut().map(|block| {
                                                    let b = Block::from_json(block.take()).unwrap();
                                                    (b.name.to_string(), b)
                                                })
                                                .collect::<HashMap<_, _>>();
        let comments = json.get_mut("Comments").ok_or("'Comments' does not exist in Visual Source json")?.take()
                                        .as_array_mut().ok_or("'Comments' in Visual Source json is not an array")?
                                        .iter_mut().map(|comment| Comment::from_json(comment.take()).unwrap())
                                        .collect::<Vec<_>>();

        Ok(Self {
            version: LATEST_VS_VERSION,
            editor,
            blocks,
            comments,
            unique_id_service: UniqueIdService::new()
        })
    }
}

impl Display for Box<dyn VSFieldType> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_vs().escape_debug())
    }
}

impl Debug for Box<dyn VSFieldType> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_vs().escape_debug())
    }
}