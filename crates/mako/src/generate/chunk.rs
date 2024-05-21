use std::fmt::{Debug, Formatter};
use std::hash::Hasher;
use std::path::{Component, Path};

use mako_core::anyhow::Result;
use mako_core::base64::engine::general_purpose;
use mako_core::base64::Engine;
use mako_core::indexmap::IndexSet;
use mako_core::md5;
use mako_core::twox_hash::XxHash64;

use crate::module::ModuleId;
use crate::module_graph::ModuleGraph;

pub type ChunkId = ModuleId;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ChunkType {
    #[allow(dead_code)]
    Runtime,
    /**
     * Entry(chunk_id, chunk_name, is_shared_chunk)
     */
    Entry(ModuleId, String, bool),
    Async,
    // mean that the chunk is not async, but it's a dependency of an async chunk
    Sync,
    // web workers
    Worker(ModuleId),
}

pub struct Chunk {
    pub id: ChunkId,
    pub chunk_type: ChunkType,
    pub modules: IndexSet<ModuleId>,
    pub content: Option<String>,
    pub source_map: Option<String>,
}

impl Debug for Chunk {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}#{}({:?})",
            self.id.id,
            self.modules.len(),
            self.chunk_type
        )?;
        Ok(())
    }
}

impl Chunk {
    pub fn new(id: ChunkId, chunk_type: ChunkType) -> Self {
        Self {
            modules: IndexSet::new(),
            id,
            chunk_type,
            content: None,
            source_map: None,
        }
    }

    pub fn filename(&self) -> String {
        match &self.chunk_type {
            ChunkType::Runtime => "runtime.js".into(),
            // foo/bar.tsx -> bar.js
            ChunkType::Entry(_, name, _) => format!("{}.js", name),
            // foo/bar.tsx -> foo_bar_tsx-async.js
            ChunkType::Async | ChunkType::Sync | ChunkType::Worker(_) => {
                let parsed_id = parse_path(&self.id.id).ok().unwrap();
                let path = Path::new(&parsed_id.path);
                let query = parsed_id
                    .query
                    .into_iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect::<Vec<String>>()
                    .join("&");

                let mut name = path
                    .components()
                    .filter(|c| !matches!(c, Component::RootDir | Component::CurDir))
                    .map(|c| match c {
                        Component::ParentDir => "@".to_string(),
                        Component::Prefix(_) => "@".to_string(),
                        Component::RootDir => "".to_string(),
                        Component::CurDir => "".to_string(),
                        Component::Normal(seg) => seg.to_string_lossy().replace(['.', '?'], "_"),
                    })
                    .collect::<Vec<String>>()
                    .join("_");

                if !query.is_empty() {
                    let query_hash =
                        general_purpose::URL_SAFE.encode(md5::compute(query).0)[..4].to_string();
                    name = format!("{}_q_{}", name, query_hash);
                }

                format!(
                    "{}-{}.js",
                    name,
                    if matches!(self.chunk_type, ChunkType::Worker(_)) {
                        "worker"
                    } else {
                        "async"
                    }
                )
            }
        }
    }

    pub fn add_module(&mut self, module_id: ModuleId) {
        if let (pos, false) = self.modules.insert_full(module_id.clone()) {
            // module already exists, move it to the back
            self.modules.shift_remove_index(pos);
            self.modules.insert(module_id);
        }
    }

    pub fn get_modules(&self) -> &IndexSet<ModuleId> {
        &self.modules
    }

    #[allow(dead_code)]
    pub fn mut_modules(&mut self) -> &mut IndexSet<ModuleId> {
        &mut self.modules
    }

    pub fn remove_module(&mut self, module_id: &ModuleId) {
        self.modules.shift_remove(module_id);
    }

    pub fn has_module(&self, module_id: &ModuleId) -> bool {
        self.modules.contains(module_id)
    }

    pub fn hash(&self, mg: &ModuleGraph) -> u64 {
        let mut sorted_module_ids = self.modules.iter().cloned().collect::<Vec<ModuleId>>();
        sorted_module_ids.sort_by_key(|m| m.id.clone());

        let mut hash: XxHash64 = Default::default();

        for id in sorted_module_ids {
            let m = mg.get_module(&id).unwrap();
            hash.write_u64(m.info.as_ref().unwrap().raw_hash);
        }

        hash.finish()
    }
}

// TODO: REMOVE THIS
fn parse_path(path: &str) -> Result<FileRequest> {
    let mut iter = path.split('?');
    let path = iter.next().unwrap();
    let query = iter.next().unwrap_or("");
    let mut query_vec = vec![];
    for pair in query.split('&') {
        if pair.contains('=') {
            let mut it = pair.split('=').take(2);
            let kv = match (it.next(), it.next()) {
                (Some(k), Some(v)) => (k.to_string(), v.to_string()),
                _ => continue,
            };
            query_vec.push(kv);
        } else if !pair.is_empty() {
            query_vec.push((pair.to_string(), "".to_string()));
        }
    }
    Ok(FileRequest {
        path: path.to_string(),
        query: query_vec,
    })
}

#[derive(Debug, Clone)]
struct FileRequest {
    pub path: String,
    pub query: Vec<(String, String)>,
}

#[cfg(test)]
mod tests {
    use crate::generate::chunk::{Chunk, ChunkType};
    use crate::module::ModuleId;

    #[test]
    fn test_filename() {
        let module_id = ModuleId::new("foo/bar.tsx".into());
        let chunk = Chunk::new(
            module_id.clone(),
            ChunkType::Entry(module_id, "foo_bar".to_string(), false),
        );
        assert_eq!(chunk.filename(), "foo_bar.js");

        let chunk = Chunk::new(ModuleId::new("./foo/bar.tsx".into()), ChunkType::Async);
        assert_eq!(chunk.filename(), "foo_bar_tsx-async.js");

        let chunk = Chunk::new(ModuleId::new("foo/bar.tsx".into()), ChunkType::Runtime);
        assert_eq!(chunk.filename(), "runtime.js");
    }
}