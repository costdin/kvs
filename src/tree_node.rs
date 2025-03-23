use std::io::BufWriter;
use std::ops::Bound::Included;
use std::str;
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{BufReader, Read, Seek, SeekFrom, Write},
    ops::Bound,
    path::PathBuf,
};

use log::{debug, error};

pub const SPLIT_THRESHOLD: usize = 8 * 1024 * 1024; // 8MB
pub const IO_BUFFER_SIZE: usize = MAX_VALUE_LEN + MAX_KEY_LEN * 2;
pub const MAX_KEY_LEN: usize = u8::MAX as usize;
pub const MAX_VALUE_LEN: usize = 32 * 1024; // 1MB
pub const METADATA_LENGTH: usize = MAX_KEY_LEN + size_of::<u8>() + size_of::<u32>() + 36; // 8KB

#[derive(Debug)]
pub enum TrieError {
    IoError(std::io::Error),
    KeyError,
    ValueError,
    WrongNode(String),
    NotFound,
}

pub struct TreeNode {
    is_leaf: Option<bool>,
    prefix: String,
    base_path: PathBuf,
    file_path: PathBuf,
    file: Option<File>,
    children: [Option<char>; 36],
    entries: Option<BTreeMap<String, String>>,
    sync_after_write: bool,
}

pub struct FindRangeChildrenResult {
    pub values: Vec<(String, String)>,
    pub child_prefixes: Vec<String>,
}

enum DeserializeResult {
    Set(String, String, usize),
    Delete(String, usize),
    IncompleteRead,
    EmptyBuffer,
}

enum Operation<'a> {
    Put { key: &'a str, value: &'a str },
    Delete { key: &'a str },
}

pub enum SearchResult {
    Current(),
    Child(String),
    NonExistingChild(String),
}

impl TreeNode {
    /// Creates a new TreeNode with a specific prefix and path
    pub fn create(
        base_path: PathBuf,
        prefix: &str,
        sync_after_write: bool,
    ) -> Result<TreeNode, std::io::Error> {
        let file_path = Self::file_name(&base_path, &prefix);

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&file_path)?;

        let mut node = TreeNode {
            is_leaf: Some(true),
            prefix: prefix.to_string(),
            file: Some(file),
            children: [const { None }; 36],
            entries: Some(BTreeMap::new()),
            file_path,
            base_path,
            sync_after_write,
        };

        node.save_metadata()?;

        Ok(node)
    }

    /// Creates a TreeNode from an existing file and loads the metadata and data as necessary
    pub fn from(
        base_path: PathBuf,
        prefix: &str,
        load_metadata: bool,
        load_data: bool,
        sync_after_write: bool,
    ) -> Result<TreeNode, std::io::Error> {
        let file_path = Self::file_name(&base_path, prefix);

        let mut node = TreeNode {
            base_path,
            file_path,
            prefix: prefix.to_string(),
            is_leaf: None,
            file: None,
            children: [const { None }; 36],
            entries: None,
            sync_after_write,
        };

        if load_metadata || load_data {
            node.read_metadata()?;
        }

        if load_data {
            node.read_data()?;
        }

        Ok(node)
    }

    /// Saves the metadata (prefix, leaf status, children) to disk
    pub fn save_metadata(&mut self) -> Result<(), std::io::Error> {
        let mut buffer = [0; METADATA_LENGTH];
        buffer[0] = self.prefix.len() as u8;
        if self.prefix.len() > 0 {
            // Root
            buffer[1..(self.prefix.len() + 1)].copy_from_slice(self.prefix.as_bytes());
        }
        buffer[MAX_KEY_LEN + 1] = if self.is_leaf.unwrap() { 1 } else { 0 };
        for (ix, c) in self.children.iter().enumerate() {
            if c.is_some() {
                buffer[MAX_KEY_LEN + 2 + ix] = 1;
            }
        }

        let file = self.file.as_mut().unwrap();
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&buffer)?;

        Ok(())
    }

    /// Retrieves a value for a given key
    pub fn get(&mut self, key: &str) -> Result<String, TrieError> {
        self.read_metadata()?;
        if !Self::is_valid_key(&key) || !self.owns_key(&key) {
            return Err(TrieError::KeyError);
        }

        self.read_data()?;

        match self.entries.as_ref().unwrap().get(key) {
            Some(r) => Ok(r.clone()),
            None => Err(TrieError::NotFound),
        }
    }

    /// Retrieves a range of values within the specified key range
    pub fn get_range(
        &mut self,
        start_key: &String,
        end_key: &String,
        limit: Option<usize>,
    ) -> Result<Vec<(String, String)>, TrieError> {
        if !Self::is_valid_key(start_key) || !Self::is_valid_key(end_key) {
            return Err(TrieError::KeyError);
        }

        self.read_metadata()?;
        self.read_data()?;

        let iterator = self
            .entries
            .as_ref()
            .unwrap()
            .range::<String, (Bound<&String>, Bound<&String>)>((
                Included(start_key),
                Included(end_key),
            ))
            .map(|(k, v)| (k.clone(), v.clone()));

        let result = match limit {
            Some(l) => iterator.take(l).collect(),
            None => iterator.collect(),
        };

        return Ok(result);
    }

    /// Inserts a key-value pair
    pub fn insert(&mut self, key: String, value: String) -> Result<(), TrieError> {
        self.read_metadata()?;
        if !Self::is_valid_key(&key) {
            return Err(TrieError::KeyError);
        }

        if !self.owns_key(&key) {
            return Err(TrieError::WrongNode(
                key[..(self.prefix.len() + 1)].to_string(),
            ));
        }

        if !Self::is_valid_value(&value) {
            return Err(TrieError::ValueError);
        }

        let operation = Operation::Put {
            key: &key,
            value: &value,
        };

        self.save_operation(operation)?;
        self.entries.as_mut().and_then(|e| e.insert(key, value));

        self.split()?;

        Ok(())
    }

    /// Deletes a key
    pub fn delete(&mut self, key: String) -> Result<(), TrieError> {
        self.read_metadata()?;
        if !Self::is_valid_key(&key) || !self.owns_key(&key) {
            return Err(TrieError::KeyError);
        }

        if self.is_leaf.unwrap() || key == self.prefix {
            if !key.starts_with(&self.prefix) {
                panic!("error!");
            }

            self.save_operation(Operation::Delete { key: &key })?;

            self.entries.as_mut().and_then(|e| e.remove(&key));
        }

        Ok(())
    }

    /// Returns a range or keys within the key boundaries and the list children that may have
    /// relevant entries
    pub fn find_range_children(
        &mut self,
        start_key: &String,
        end_key: &String,
        limit: Option<usize>,
    ) -> Result<FindRangeChildrenResult, TrieError> {
        if !Self::is_valid_key(start_key) || !Self::is_valid_key(end_key) {
            return Err(TrieError::KeyError);
        }

        let values = if self.is_leaf.unwrap() || *start_key <= self.prefix {
            self.get_range(start_key, end_key, limit)?
        } else {
            vec![]
        };

        let mut child_prefixes = vec![];

        if !self.is_leaf.unwrap() {
            let start_ix = match (start_key, &self.prefix) {
                (s, p) if s.len() <= p.len() && s <= p => Some(0),
                (s, p) if s.len() <= p.len() => None,
                (s, p) => {
                    let child_prefix = &s[0..=p.len()];
                    Some(Self::last_char_to_index(child_prefix))
                }
            };

            let end_ix = match (end_key, &self.prefix) {
                (e, p) if e.len() <= p.len() && e >= p => Some(self.children.len() - 1),
                (e, p) if e.len() <= p.len() => None,
                (e, p) => {
                    let child_prefix = &e[0..=p.len()];
                    Some(Self::last_char_to_index(child_prefix))
                }
            };

            // these options should never be empty
            if let (Some(s), Some(e)) = (start_ix, end_ix) {
                for ix in s..=e {
                    if self.children[ix].is_some() {
                        let mut cp = self.prefix.clone();
                        cp.push(Self::index_to_char(ix));
                        child_prefixes.push(cp);
                    }
                }
            }
        }

        Ok(FindRangeChildrenResult {
            values,
            child_prefixes,
        })
    }

    pub fn get_children_prefixes(&self) -> Vec<String> {
        let mut child_prefixes = vec![];

        for ix in 0..self.children.len() {
            if self.children[ix].is_some() {
                let mut cp = self.prefix.clone();
                cp.push(Self::index_to_char(ix));
                child_prefixes.push(cp);
            }
        }

        child_prefixes
    }

    /// Returns the prefix of the node
    pub fn prefix(&self) -> &String {
        &self.prefix
    }

    /// Registers a new child in the node (used when new child nodes are created)
    pub fn register_child(&mut self, prefix: String) {
        let ix = Self::last_char_to_index(&prefix[0..=self.prefix.len()]);
        self.register_child_int(ix);
    }

    /// Returns `SearchResult::Current` if the node owns the key. Otherwise returns the prefix
    /// of a child that owns the node
    pub fn find_owner(&self, key: &str) -> SearchResult {
        if self.owns_key(key) {
            SearchResult::Current()
        } else {
            let child_prefix = &key[0..=self.prefix.len()];
            let ix = Self::last_char_to_index(child_prefix);
            match &self.children[ix] {
                Some(c) => {
                    let mut prefix = self.prefix.clone();
                    prefix.push(*c);
                    SearchResult::Child(prefix)
                }
                None => {
                    let p = child_prefix.to_string();
                    SearchResult::NonExistingChild(p)
                }
            }
        }
    }

    /// Returns true if the data of the node has been retrieved from disk
    pub fn has_data(&self) -> bool {
        self.entries.is_some()
    }

    fn read_metadata(&mut self) -> Result<(), std::io::Error> {
        if self.has_metadata() {
            return Ok(());
        }

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.file_path)?;

        let mut buffer = [0; METADATA_LENGTH];
        file.read(&mut buffer).unwrap();

        let prefix_len = buffer[0] as usize;

        for ix in 0..self.children.len() {
            if buffer[MAX_KEY_LEN + 2 + ix] == 1 {
                self.children[ix] = Some(Self::index_to_char(ix));
            }
        }

        self.prefix = str::from_utf8(&buffer[1..(prefix_len + 1)])
            .unwrap()
            .to_string();
        self.file = Some(file);
        self.is_leaf = Some(if buffer[MAX_KEY_LEN + 1] == 1 {
            true
        } else {
            false
        });

        Ok(())
    }

    fn read_data(&mut self) -> Result<(), std::io::Error> {
        if self.has_data() {
            return Ok(());
        }

        let file = self.file.as_mut().unwrap();
        file.seek(SeekFrom::Start(METADATA_LENGTH as u64))?;
        let mut entries = BTreeMap::new();

        let mut buffer = [0; IO_BUFFER_SIZE];
        let mut reader = BufReader::new(&*file);
        let mut buffer_read_position = 0;
        let mut buffer_write_position = 0;
        let mut need_fix = false;

        while let Ok(bytes_read) = reader.read(&mut buffer[buffer_write_position..]) {
            if bytes_read == 0 {
                if buffer_write_position > 0 {
                    error!(
                        "Invalid entry {:#?}\nAn entry was not fully committed.",
                        &buffer[..buffer_write_position]
                    );
                    need_fix = true;
                }

                break;
            }

            let internal_buffer = &mut buffer[..(buffer_write_position + bytes_read)];

            loop {
                match Self::deserialize(&internal_buffer[buffer_read_position..]) {
                    DeserializeResult::Set(key, value, position) => {
                        if !key.starts_with(&self.prefix) {
                            panic!("File is corrupted!");
                        }

                        entries.insert(key, value);
                        buffer_read_position += position;
                    }
                    DeserializeResult::Delete(key, position) => {
                        if !key.starts_with(&self.prefix) {
                            panic!("File is corrupted!");
                        }

                        entries.remove(&key);
                        buffer_read_position += position;
                    }
                    DeserializeResult::IncompleteRead => {
                        if buffer_read_position == 0 {
                            buffer_write_position = internal_buffer.len();
                        } else {
                            let left = buffer_read_position;
                            let right = internal_buffer.len() - buffer_read_position;

                            if left >= right {
                                let (b1, b2) = internal_buffer.split_at_mut(buffer_read_position);
                                b1[0..b2.len()].copy_from_slice(&b2);
                            } else {
                                let mut b = [0; IO_BUFFER_SIZE];
                                b[0..right]
                                    .copy_from_slice(&internal_buffer[buffer_read_position..]);
                                buffer[0..right].copy_from_slice(&b[0..right]);
                            }
                            buffer_write_position = right;
                            buffer_read_position = 0;
                        }

                        break;
                    }
                    DeserializeResult::EmptyBuffer => {
                        buffer_write_position = 0;
                        buffer_read_position = 0;

                        break;
                    }
                }
            }
        }

        self.entries = Some(entries);

        if need_fix {
            self.flush_to_disk()?;
        }

        Ok(())
    }

    fn owns_key(&self, key: &str) -> bool {
        if !self.is_leaf.unwrap() {
            self.prefix == key
        } else {
            key.starts_with(&self.prefix)
        }
    }

    fn last_char_to_index(str: &str) -> usize {
        let ix = match str.chars().last().unwrap() as u8 {
            n @ b'0'..=b'9' => n - b'0',
            l @ b'a'..=b'z' => l - b'a' + 10,
            l @ b'A'..=b'Z' => l - b'A' + 10,
            _ => panic!("no"),
        };

        ix as usize
    }

    fn index_to_char(ix: usize) -> char {
        match ix {
            0..=9 => (ix as u8 + b'0') as char,
            10..36 => (ix as u8 - 10 + b'a') as char,
            _ => panic!("no"),
        }
    }

    fn index_to_range(ix: usize) -> (char, char) {
        match ix {
            0..=9 => ((ix as u8 + b'0') as char, (ix as u8 + b'1') as char),
            10..36 => (
                (ix as u8 - 10 + b'a') as char,
                (ix as u8 - 10 + b'b') as char,
            ),
            _ => panic!("no"),
        }
    }

    fn has_metadata(&self) -> bool {
        self.file.is_some() && self.is_leaf.is_some()
    }

    fn set_entries(&mut self, entries: BTreeMap<String, String>) -> Result<(), std::io::Error> {
        self.entries = Some(entries);
        self.flush_to_disk()?;

        Ok(())
    }

    fn save_operation(&mut self, operation: Operation) -> Result<(), std::io::Error> {
        let mut buffer = [0u8; IO_BUFFER_SIZE];
        let total_length = Self::serialize(&mut buffer, operation).unwrap();

        let file = self.file.as_mut().unwrap();
        file.seek(SeekFrom::End(0))?;

        if file.stream_position()? < METADATA_LENGTH as u64 {
            file.seek(SeekFrom::Start(METADATA_LENGTH as u64))?;
        }

        let mut buf_writer = BufWriter::new(file);
        buf_writer.write(&buffer[0..total_length])?;
        buf_writer.flush()?;

        if self.sync_after_write {
            buf_writer.get_ref().sync_all()?;
        }

        Ok(())
    }

    fn deserialize(buffer: &[u8]) -> DeserializeResult {
        if buffer.len() == 0 {
            return DeserializeResult::EmptyBuffer;
        }

        if buffer.len() < 2 {
            return DeserializeResult::IncompleteRead;
        }

        let operation_type = buffer[0];
        let key_len = buffer[1] as usize;
        if key_len + 2 > buffer.len() {
            return DeserializeResult::IncompleteRead;
        }

        let key = str::from_utf8(&buffer[2..(key_len + 2)])
            .unwrap()
            .to_string();

        // DELETE
        if operation_type == 1 {
            DeserializeResult::Delete(key, key_len + 2)
        } else {
            if key_len + 6 > buffer.len() {
                return DeserializeResult::IncompleteRead;
            }

            let value_len =
                u32::from_le_bytes(buffer[key_len + 2..key_len + 6].try_into().unwrap()) as usize;
            let total_len = key_len + value_len + 6;
            if total_len > buffer.len() {
                DeserializeResult::IncompleteRead
            } else {
                let value = str::from_utf8(&buffer[(key_len + 6)..total_len])
                    .unwrap()
                    .to_string();

                DeserializeResult::Set(key, value, total_len)
            }
        }
    }

    fn serialize(buffer: &mut [u8], operation: Operation) -> Option<usize> {
        let total_length = match &operation {
            Operation::Put { key, value } => key.len() + value.len() + 6,
            Operation::Delete { key } => key.len() + 2,
        };

        if total_length > buffer.len() {
            None
        } else {
            match operation {
                Operation::Put { key, value } => {
                    buffer[0] = 0;
                    buffer[1] = key.len() as u8;
                    buffer[key.len() + 2..key.len() + 6]
                        .copy_from_slice(&u32::to_le_bytes(value.len() as u32));
                    buffer[2..(key.len() + 2)].copy_from_slice(key.as_bytes());
                    buffer[(key.len() + 6)..total_length].copy_from_slice(value.as_bytes());
                }
                Operation::Delete { key } => {
                    buffer[0] = 1;
                    buffer[1] = key.len() as u8;
                    buffer[2..(key.len() + 2)].copy_from_slice(key.as_bytes());
                }
            };

            Some(total_length)
        }
    }

    fn flush_to_disk(&mut self) -> Result<(), std::io::Error> {
        if !self.has_data() {
            debug!("Trying to flush empty page");

            return Ok(());
        }

        let mut total_written = 0;

        {
            let file = self.file.as_mut().unwrap();
            file.seek(SeekFrom::Start(METADATA_LENGTH as u64))?;

            let mut buf_writer = BufWriter::new(file);

            let mut buffer = [0u8; IO_BUFFER_SIZE];

            for (key, value) in self.entries.iter().flatten() {
                let size = Self::serialize(
                    &mut buffer,
                    Operation::Put {
                        key: &key,
                        value: &value,
                    },
                )
                .unwrap();
                total_written += size;
                buf_writer.write(&buffer[..size])?;
            }

            buf_writer.flush()?;
            buf_writer.get_ref().sync_all()?;
        }
        let file = self.file.as_mut().unwrap();
        file.set_len((METADATA_LENGTH + total_written) as u64)?;

        Ok(())
    }

    fn register_child_int(&mut self, index: usize) {
        self.children[index] = Some(Self::index_to_char(index));
    }

    fn split(&mut self) -> Result<(), std::io::Error> {
        let file_size = self
            .file
            .as_ref()
            .map(|f| f.metadata().unwrap().len())
            .unwrap_or(0) as usize;
        let mut transferred = 0;

        if file_size > METADATA_LENGTH && file_size - METADATA_LENGTH > SPLIT_THRESHOLD {
            self.read_data()?;
            let count = self.entries.as_ref().unwrap().len();

            for i in (0..36).rev() {
                let (low, high) = Self::index_to_range(i);

                let mut prefix = self.prefix.clone();
                prefix.push(low);
                let mut highf = self.prefix.clone();
                highf.push(high);

                let entries = self.entries.as_mut().unwrap().split_off(&prefix);

                if entries.len() > 0 {
                    transferred += entries.len();

                    let mut node =
                        TreeNode::create(self.base_path.clone(), &prefix, self.sync_after_write)?;
                    node.set_entries(entries)?;
                    self.children[i] = Some(low);
                }
            }

            if self.entries.as_ref().unwrap().len() > 1 {
                panic!("Failed to split page");
            }

            if transferred + self.entries.as_ref().unwrap().len() != count {
                panic!("Failed to split page");
            }

            self.is_leaf = Some(false);
            self.save_metadata()?;
        }

        Ok(())
    }

    fn is_valid_key(key: &str) -> bool {
        key.len() <= MAX_KEY_LEN && key.chars().all(char::is_alphanumeric)
    }

    fn is_valid_value(value: &str) -> bool {
        value.len() <= MAX_VALUE_LEN
    }

    fn file_name(base_path: &PathBuf, prefix: &str) -> PathBuf {
        if prefix.len() == 0 {
            // root
            base_path.join("_root.dat")
        } else {
            base_path.join(format!("{prefix}.dat"))
        }
    }
}

impl From<std::io::Error> for TrieError {
    fn from(e: std::io::Error) -> Self {
        TrieError::IoError(e)
    }
}
