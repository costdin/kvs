use log::debug;

use crate::{
    cache::Cache,
    tree_node::{self, FindRangeChildrenResult, SearchResult, TreeNode, TrieError},
};
use std::{collections::HashMap, path::PathBuf};

pub struct NodeReader {
    metadata_cache: Cache<String, TreeNode>,
    data_cache: Cache<String, TreeNode>,
    root: TreeNode,
    base_path: PathBuf,
    max_range_response_size: Option<usize>,
    sync_after_write: bool,
}

impl NodeReader {
    /// Instantiates a new NodeReader
    pub fn new(
        base_path: PathBuf,
        cache_size: usize,
        max_range_response_size: Option<usize>,
        sync_after_write: bool,
    ) -> Result<NodeReader, std::io::Error> {
        Ok(NodeReader {
            root: Self::read_root(&base_path, sync_after_write)?,
            data_cache: Cache::new(cache_size / tree_node::SPLIT_THRESHOLD),
            metadata_cache: Cache::new(10000),
            base_path,
            max_range_response_size,
            sync_after_write,
        })
    }

    /// Removes an entry
    pub fn delete(&mut self, key: String) -> Result<(), TrieError> {
        self.on_owner(&key.clone(), |n| {
            n.delete(key)?;
            Ok(())
        })
    }

    /// Runs a sanity check (opens all partitions)
    pub fn sanity_check(&mut self) -> Result<(), std::io::Error> {
        let mut nodes = self.root.get_children_prefixes();

        while nodes.len() > 0 {
            let node_prefix = nodes.pop().unwrap();
            debug!("Checking: {node_prefix}");

            let node = TreeNode::from(
                self.base_path.clone(),
                &node_prefix,
                true,
                true,
                self.sync_after_write,
            )?;

            nodes.append(&mut node.get_children_prefixes());
        }

        Ok(())
    }

    /// Returns a list of entries whose keys are withing the given range
    pub fn get_range(
        &mut self,
        start_key: &String,
        end_key: &String,
    ) -> Result<Vec<(String, String)>, TrieError> {
        let FindRangeChildrenResult {
            values: mut result,
            child_prefixes: mut nodes,
        } = self
            .root
            .find_range_children(start_key, end_key, self.max_range_response_size)?;

        nodes.reverse();

        while nodes.len() > 0 && result.len() < self.max_range_response_size.unwrap_or(usize::MAX) {
            let limit = self.max_range_response_size.map(|l| l - result.len());
            let node_prefix = nodes.pop().unwrap();
            let mut r = self.on_owner(&node_prefix, |n| {
                n.find_range_children(start_key, end_key, limit)
            })?;

            r.child_prefixes.reverse();

            result.append(&mut r.values);
            nodes.append(&mut r.child_prefixes);
        }

        Ok(result)
    }

    /// Inserts an entry
    pub fn insert(&mut self, mut key: String, value: String) -> Result<(), TrieError> {
        key = key.to_lowercase();

        self.on_owner(&key.clone(), |n| {
            n.insert(key.to_lowercase(), value)?;
            Ok(())
        })
    }

    /// Bulk inserts a list of entries
    pub fn bulk_insert(&mut self, entries: HashMap<String, String>) -> Result<(), TrieError> {
        for (key, value) in entries {
            self.insert(key, value)?;
        }

        Ok(())
    }

    /// Returns the value of an entry
    pub fn get(&mut self, key: &str) -> Result<String, TrieError> {
        self.on_owner(key, move |n| n.get(key))
    }

    fn read_root(base_path: &PathBuf, sync_after_write: bool) -> Result<TreeNode, std::io::Error> {
        let root = match TreeNode::from(base_path.clone(), "", true, true, sync_after_write) {
            Ok(r) => r,
            Err(_) => TreeNode::create(base_path.clone(), "", sync_after_write)?,
        };

        Ok(root)
    }

    /// Iterates over the tree structure to find the owning node, then executed an operation against it
    /// Used by all other methods in this struct
    fn on_owner<T, U: FnOnce(&mut TreeNode) -> Result<T, TrieError>>(
        &mut self,
        key: &str,
        func: U,
    ) -> Result<T, TrieError> {
        let mut node = &mut self.root;
        let mut traversed_nodes = vec![];
        loop {
            node = match node.find_owner(&key) {
                SearchResult::Current() => {
                    break;
                }
                SearchResult::Child(prefix) => {
                    if let Some(entry) = self
                        .data_cache
                        .remove(&prefix.to_string())
                        .or(self.metadata_cache.remove(&prefix.to_string()))
                    {
                        traversed_nodes.push(entry);
                        traversed_nodes.last_mut().unwrap()
                    } else {
                        debug!("Cache miss: {prefix}");
                        traversed_nodes.push(TreeNode::from(
                            self.base_path.clone(),
                            &prefix,
                            true,
                            false,
                            self.sync_after_write,
                        )?);
                        traversed_nodes.last_mut().unwrap()
                    }
                }
                SearchResult::NonExistingChild(prefix) => {
                    let n =
                        TreeNode::create(self.base_path.clone(), &prefix, self.sync_after_write)?;
                    node.register_child(prefix.clone());
                    node.save_metadata()?;

                    traversed_nodes.push(n);
                    traversed_nodes.last_mut().unwrap()
                }
            };
        }

        let r = func(node);

        for node in traversed_nodes.into_iter() {
            if node.has_data() && node.prefix() != "" {
                self.data_cache.set(node.prefix().to_string(), node);
            } else {
                self.metadata_cache.set(node.prefix().to_string(), node);
            }
        }

        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_node_reader_creation() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_path_buf();
        let reader = NodeReader::new(path, 10, Some(1000), false);

        assert!(reader.is_ok());
    }

    #[test]
    fn test_node_reader_cache_retrieval() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_path_buf();
        let mut reader = NodeReader::new(path, 10, Some(1000), false).unwrap();

        for i in 0..100000 {
            reader
                .insert(format!("key{i:0>8}"), format!("value{i:0>8}"))
                .unwrap();
            let read_result = reader.get(&format!("key{i:0>8}")).unwrap();
            assert_eq!(read_result, format!("value{i:0>8}"));
        }
    }

    #[test]
    fn test_get_range() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_path_buf();
        let mut reader = NodeReader::new(path, 10, None, false).unwrap();

        for i in 0..100000 {
            reader
                .insert(format!("key{i:0>8}"), format!("value{i:0>8}"))
                .unwrap();
        }

        assert_eq!(
            reader
                .get_range(&"key00090000".to_string(), &"z".to_string())
                .unwrap()
                .len(),
            10000
        );
    }

    #[test]
    fn test_get_range_limit() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_path_buf();
        let mut reader = NodeReader::new(path, 10, Some(1000), false).unwrap();

        for i in 0..100000 {
            reader
                .insert(format!("key{i:0>8}"), format!("value{i:0>8}"))
                .unwrap();
        }

        assert_eq!(
            reader
                .get_range(&"key00090000".to_string(), &"z".to_string())
                .unwrap()
                .len(),
            1000
        );
    }
}
