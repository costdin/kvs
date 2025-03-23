# Key-Value Store

This is a simple key-value store implemented using a Trie-like structure. It supports basic CRUD operations and is optimized for performance and flexibility, with multiple configuration options available for data commitment strategies and node roles. The store uses the file system to persist data and provides endpoints for both single-value and bulk operations.

## Table of Contents

- [Running with Docker](#running-with-docker)
- [Operations](#operations)
- [Constraints](#constraints)
- [File System Storage](#file-system-storage)
- [Data Commitment Strategies](#data-commitment-strategies)
- [Node Roles](#node-roles)
- [API Endpoints](#api-endpoints)
- [Configuration](#configuration)
- [To Do](#todo)

## Running with Docker

You can run the key-value store using Docker Compose. The `docker-compose.yml` file will set up both the main node and a read replica.

### Steps:
1. Clone or download the project.
2. Open a terminal and navigate to the project directory.
3. Run the following command to build and start the services:
   ```bash
   docker-compose up --build
   ```
    _(It may take a few minutes—Rust is slow at building.)_

This will start:
- The main node on port `3030` (read/write).
- The read replica on port `3031` (read-only) and internal communication on port `3040`.

## Operations

The following operations are supported by the key-value store:

### GET /kv/{key}
Retrieves the value associated with a given key.

- **Request:**
  - `GET /kv/{key}`
  
- **Response:**
  - Returns the value stored for the key, or 404 if the key does not exist.

### POST /kv/{key}
Inserts or updates a key-value pair in the store.

- **Request:**
  - `POST /kv/{key}`
  - Request body should contain the value (e.g., JSON, plain text).
  
- **Response:**
  - Returns a success message on insertion.

### DELETE /kv/{key}
Deletes the key-value pair from the store.

- **Request:**
  - `DELETE /kv/{key}`
  
- **Response:**
  - Returns a success message or a 404 if the key does not exist.

### POST /bulk
Inserts multiple key-value pairs in one request. The request body should contain a map of key-value pairs.

- **Request:**
  - `POST /bulk`
  - Request body should be a JSON object containing multiple key-value pairs.
  
- **Response:**
  - Returns a success message for the bulk insert operation.

### GET /bulk/range?start_key={start_key}&end_key={end_key}
Retrieves a range of key-value pairs based on the provided `start_key` and `end_key`.

- **Request:**
  - `GET /bulk/range?start_key={start_key}&end_key={end_key}`
  
- **Response:**
  - Returns key-value pairs in the specified range.

## Constraints
- Keys
  - Must be alphanumeric (letters and numbers only).
  - Maximum length: 255 characters.
  - Case-insensitive (e.g., Key123 and key123 are treated the same).

- Values
  - No character restrictions.
  - Maximum size: 32KB.

## File System Storage

The data in the key-value store is stored using a Trie-like structure, where each node corresponds to a file on the disk. These files act as transaction logs, and each write operation is appended to the corresponding file. This approach ensures that write operations are fast and efficient, as we don't need to open or check the files before writing; we simply append to the end of the log.

### Transaction Logs

- Each file serves as a transaction log for the node it represents in the Trie.
- When a write operation (like an insert or update) is performed, the data is appended to the appropriate log file.
- This append-only design allows write operations to be performed quickly, as no complex file opening or checking is required before writing.

### Page Splitting and Child Partitions
When a node grows too large and exceeds its capacity, it triggers a page split. During this process:
- Data is ordered to ensure consistency.
- The data is then moved into child partitions, corresponding to new files.
- The Trie structure ensures that the key-value pairs are organized efficiently for both lookups and write operations.

### Key Format and File Naming
- Key format: Keys must be alphanumeric (letters and numbers).
- File Naming: Files are named after their key prefixes (e.g., abc.dat, def.dat), and each file stores data for a specific range of keys within the Trie structure.
- The root node of the Trie is stored in a file named `_root.dat`. As new data is added, the Trie expands and creates new files for each node.

### Data Commitment Strategies

The system offers two data commitment strategies to control when data is flushed to disk:

1. **Default**  
   - Data is written to disk in the background as part of the normal file system flush operation. This is faster but might delay persistence of updates.
   
2. **Strict**  
   - Data is immediately flushed to disk after each write operation. This is slower but ensures that changes are persisted to disk immediately after each write.

You can configure the store to use one of these strategies based on your performance and consistency requirements.

## Node Roles

This key-value store can be run in two different roles:

### Main Node
- Exposes both read and write operations on port `3030`.
- Responsible for data modifications (insertions, deletions, etc.).

### Read Replica
- Exposes only read operations on port `3030`.
- Exposes write operations on port `3040` for internal communication between nodes (only accessible within the Docker network).

**Port Accessibility:**
- Main node: `3030` for read and write operations.
- Read replica: `3030` for read-only operations, `3040` for internal write operations.

#### API Endpoints

#### Main Node (Port 3030)
- **GET** `/kv/{key}`: Retrieve a value by key (read operation).
- **POST** `/kv/{key}`: Insert or update a key-value pair (write operation).
- **DELETE** `/kv/{key}`: Delete a key-value pair (write operation).
- **POST** `/bulk`: Insert multiple key-value pairs (write operation).
- **GET** `/bulk/range?start_key={start_key}&end_key={end_key}`: Retrieve a range of key-value pairs (read operation).

#### Read Replica (Port 3031)
- **GET** `/kv/{key}`: Retrieve a value by key (read operation).
- **GET** `/bulk/range?start_key={start_key}&end_key={end_key}`: Retrieve a range of key-value pairs (read operation).

#### Internal Write (Port 3040)
- **POST** `/kv/{key}`: Insert or update a key-value pair (write operation).
- **DELETE** `/kv/{key}`: Delete a key-value pair (write operation).
- **POST** `/bulk`: Insert multiple key-value pairs (write operation).

## Configuration  

The application reads its configuration from `config.json`. Below are the available settings and their descriptions:  

```json
{
    "max_range_response": 1000,
    "fsync": "default|strict",
    "port": 3030,
    "replication_port": 3040,
    "cache_size": 500,
    "is_replica": true,
    "replicas": ["http://kvs-replica:3040"]
}
```  

### Configuration Options  

- **`max_range_response`** *(integer, default: `1000`)*  
  - Defines the maximum number of entries returned in response to a range query.  
  - This prevents excessive data retrieval in a single request.

- **`fsync`** *(string, default: `"default"`)*  
  - Determines the data commitment strategy.  
  - Options:  
    - `"default"`: Relies on the operating system’s file system flush behavior.  
    - `"strict"`: Flushes data to disk immediately after every write (slower but safer).  

- **`port`** *(integer, default: `3030`)*  
  - Defines the port on which the node listens for client requests (read and write operations for a main node, read-only for a replica).  

- **`replication_port`** *(integer, default: `3040`)*  
  - If the node is a **replica**, it will listen for **write operations** on this port.  
  - Ignored for main nodes.  

- **`cache_size`** *(integer, default: `500`)*  
  - The size of the in-memory cache in MB.  
  - Helps improve performance by reducing disk reads for frequently accessed keys.  

- **`is_replica`** *(boolean, default: `false`)*  
  - If set to `true`, the node functions as a **replica**.  
  - Replicas do not expose write operations on their default port (`3030`).  

- **`replicas`** *(array of strings, default: `[]`)*  
  - A list of **replica node URLs** for clustering and replication.  
  - Example: `["http://kvs-replica:3040"]`  

---

## TODO

### 1. **Storage Optimization**
Currently, the key-value store relies on the file system to manage nodes, which is not the most efficient solution. In the future, the storage system should be restructured to use a better file format, minimizing reliance on the file system. This should improve performance and scalability.

### 2. **Keys and Values limitations**
Keys are currently case-insensitive and restricted to alphanumeric characters. In the future, all Unicode characters should be supported for keys. Additionally, larger values should also be supported.

### 3. **Additional Data Commitment Strategies**
There are currently two data commitment strategies: **Default** (background file system flush) and **Strict** (immediate flush after each write). More strategies should be introduced, such as:
- **fsync after N operations**: Flush data after a certain number of write operations.
- **fsync after N seconds**: Flush data after a specified time interval.

These additional strategies will allow users to choose the most suitable approach based on their consistency and performance needs.

### 4. **Clustering Enhancements**
The current clustering approach relies on a function that waits for events and sends HTTP requests to child nodes. While this works, it is not production-ready and lacks several important features:
- **Retries**: The current clustering logic does not handle retries in case of network failures or other issues.
- **Improved synchronization**: The clustering mechanism should be made more robust to handle various edge cases and ensure that data is properly synchronized across nodes.

### 5. **Error Handling**
Error handling across the system should be improved. Currently, the system may not handle certain types of errors gracefully, particularly in edge cases like network failures, file system issues, or invalid inputs. Improved error handling will ensure the system is more robust and provides clearer feedback to users.

### 6. **Sharding Support**
Support for **sharding** should be added to allow the system to scale horizontally. Each instance should be able to redirect requests to the appropriate node that owns the relevant data partition. Otherwise, this could be implemented with an "intelligent" proxy that dynamically determines where the data is located.

### 7. **Multi-Threading**
The application is single-threaded but still achieves a good level of performance due to its efficient append-only storage model and lightweight request handling (in my local setup, up to 10K write operations per second with the **Default** commit strategy, and 100s of requests per second with **Strict**). While this design keeps things simple and avoids concurrency issues, future work could explore using asynchronous I/O operations to improve efficiency without introducing full multi-threading, and per-page locking to allow parallel executions.

As with **sharding**, some of these objectives could already be achieved with an "intelligent" proxy, distributing the load across several nodes, with some read replicas. 

I've refined your **Memory Management** section for better clarity and readability:  

### **8. Memory Management**  

The application caches data in memory to improve performance. The number of pages kept in memory is determined by:  

```
max_cache_size / (max_page_size * 3.5)
```

This formula approximates the actual memory footprint of a page stored in a `BTreeMap`, as the in-memory representation is larger than its serialized form on disk.  

Potential improvements:
- **Adaptive tuning**: The system could dynamically adjust cache size based on observed memory pressure rather than relying on a static formula.  
- **Improve accuracy**: The current approximation may not always reflect real-world memory consumption, especially under varying workloads.

### 9. **Clean-up**
The code is messy in some areas and require some clean-up.