# ViOS API Reference

> Complete API documentation for ViOS kernel interfaces, syscalls, and trait definitions

## Table of Contents

1. [Overview](#overview)
2. [Syscall Interface](#syscall-interface)
3. [Filesystem API](#filesystem-api)
4. [Block Device API](#block-device-api)
5. [Driver API](#driver-api)
6. [IPC Primitives](#ipc-primitives)
7. [Task Management](#task-management)
8. [Error Handling](#error-handling)
9. [Usage Examples](#usage-examples)

---

## Overview

ViOS provides a minimal, well-defined API surface for Cells to interact with the kernel and system services. The API is designed with these principles:

- **Type Safety**: Rust's type system prevents invalid API usage
- **Zero-Copy**: Ownership-based data transfer for performance
- **Trait-Based**: Abstract interfaces allow multiple implementations
- **Stable ABI**: `#[repr(C)]` ensures binary compatibility

### API Layers

```
┌─────────────────────────────────────┐
│         Cell Application            │
├─────────────────────────────────────┤
│      libs/ostd (syscall wrappers)   │  ← User-space library
├─────────────────────────────────────┤
│      libs/api (trait definitions)   │  ← ABI contract
├─────────────────────────────────────┤
│      Kernel (trait implementations) │  ← Kernel space
└─────────────────────────────────────┘
```

---

## Syscall Interface

### Syscall Numbers

Syscalls are the only way for Cells to interact with the kernel. ViOS has a minimal set of ~15 syscalls (compared to Linux's 300+).

**Syscall Enumeration** (`libs/api/src/syscall.rs`):

```rust
#[repr(usize)]
pub enum ViSyscall {
    // === IPC (0-9) ===
    Send = 0,       // Send message to task
    Recv = 1,       // Receive message
    Call = 2,       // Synchronous RPC
    Reply = 3,      // Reply to caller

    // === Process Management (10-49) ===
    Spawn = 5,      // Create new task
    Exec = 6,       // Replace current task
    Exit = 60,      // Terminate task
    Yield = 104,    // Voluntary context switch

    // === Logging (50-59) ===
    Log = 11,       // Kernel logging

    // === Filesystem (100-199) ===
    Open = 101,     // Open file
    Read = 102,     // Read from file
    Close = 103,    // Close file
    ReadDir = 105,  // Read directory entry
    Write = 109,    // Write to file
}
```

### Syscall Invocation

**From Rust** (`libs/ostd/src/syscall.rs`):

```rust
#[inline(always)]
pub unsafe fn syscall1(syscall_id: usize, arg1: usize) -> isize {
    let ret: isize;

    #[cfg(target_arch = "riscv64")]
    core::arch::asm!(
        "ecall",
        in("a7") syscall_id,
        in("a0") arg1,
        lateout("a0") ret
    );

    ret
}

#[inline(always)]
pub unsafe fn syscall3(syscall_id: usize, arg1: usize, arg2: usize, arg3: usize) -> isize {
    let ret: isize;

    #[cfg(target_arch = "riscv64")]
    core::arch::asm!(
        "ecall",
        in("a7") syscall_id,
        in("a0") arg1,
        in("a1") arg2,
        in("a2") arg3,
        lateout("a0") ret
    );

    ret
}
```

### Syscall Calling Convention

**RISC-V**:
- `a7` (x17): Syscall number
- `a0-a5` (x10-x15): Arguments (up to 6)
- `a0` (x10): Return value (error code or handle)

**ARM**:
- `x8`: Syscall number
- `x0-x5`: Arguments
- `x0`: Return value

**x86_64**:
- `rax`: Syscall number
- `rdi, rsi, rdx, r10, r8, r9`: Arguments
- `rax`: Return value

---

## Filesystem API

### ViFileSystem Trait

**Location**: `libs/api/src/fs.rs`

**Purpose**: Abstract filesystem operations for any filesystem implementation (FAT32, ext4, TFS, etc.)

```rust
pub trait ViFileSystem: Send + Sync {
    /// Open a file at the given path.
    ///
    /// # Arguments
    /// * `path` - File path (absolute or relative to CWD)
    /// * `mode` - Open mode (Read, Write, ReadWrite)
    ///
    /// # Returns
    /// * `Ok(Box<dyn ViFile>)` - File handle on success
    /// * `Err(ViError)` - Error if file not found or permission denied
    fn open(&self, path: &str, mode: OpenMode) -> ViResult<Box<dyn ViFile + Send + Sync>>;

    /// Create a directory.
    ///
    /// # Arguments
    /// * `path` - Directory path to create
    ///
    /// # Returns
    /// * `Ok(())` - Success
    /// * `Err(ViError::AlreadyExists)` - Directory already exists
    /// * `Err(ViError::NotFound)` - Parent directory not found
    fn mkdir(&self, path: &str) -> ViResult<()>;

    /// Remove a file or directory.
    ///
    /// # Arguments
    /// * `path` - Path to remove
    ///
    /// # Returns
    /// * `Ok(())` - Success
    /// * `Err(ViError::NotFound)` - Path does not exist
    /// * `Err(ViError::NotEmpty)` - Directory not empty
    fn remove(&self, path: &str) -> ViResult<()>;
}
```

### ViFile Trait

**File Operations**:

```rust
pub trait ViFile: Send + Sync {
    /// Read data into buffer.
    ///
    /// # Arguments
    /// * `buf` - Buffer to read into
    ///
    /// # Returns
    /// * `Ok(usize)` - Number of bytes read (0 = EOF)
    /// * `Err(ViError)` - I/O error
    fn read(&mut self, buf: &mut [u8]) -> ViResult<usize>;

    /// Write data from buffer.
    ///
    /// # Arguments
    /// * `buf` - Data to write
    ///
    /// # Returns
    /// * `Ok(usize)` - Number of bytes written
    /// * `Err(ViError)` - I/O error or disk full
    fn write(&mut self, buf: &[u8]) -> ViResult<usize>;

    /// Seek to position in file.
    ///
    /// # Arguments
    /// * `pos` - Seek position (Start/End/Current + offset)
    ///
    /// # Returns
    /// * `Ok(u64)` - New position from start of file
    /// * `Err(ViError)` - Seek beyond file bounds
    fn seek(&mut self, pos: SeekFrom) -> ViResult<u64>;

    /// Check if this handle represents a directory.
    fn is_dir(&self) -> bool { false }

    /// Read next directory entry (if this is a directory).
    ///
    /// # Returns
    /// * `Ok(Some(DirEntry))` - Next entry
    /// * `Ok(None)` - End of directory
    /// * `Err(ViError::NotSupported)` - Not a directory
    fn read_dir(&mut self) -> ViResult<Option<DirEntry>> {
        Err(ViError::NotSupported)
    }
}
```

### Data Types

**OpenMode**:
```rust
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum OpenMode {
    Read,       // O_RDONLY
    Write,      // O_WRONLY | O_CREAT | O_TRUNC
    ReadWrite,  // O_RDWR
}
```

**SeekFrom**:
```rust
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum SeekFrom {
    Start(u64),     // Absolute position from start
    End(i64),       // Offset from end (negative = before end)
    Current(i64),   // Offset from current position
}
```

**DirEntry** (`libs/types/src/lib.rs`):
```rust
#[repr(C)]
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: [u8; 256],   // Null-terminated filename
    pub file_type: FileType,
    pub size: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    File,
    Directory,
    Symlink,
}
```

### Usage Example

```rust
use api::fs::{ViFileSystem, OpenMode, SeekFrom};

fn read_config(fs: &dyn ViFileSystem) -> Result<String, ViError> {
    // Open file
    let mut file = fs.open("/etc/config.toml", OpenMode::Read)?;

    // Read contents
    let mut buffer = vec![0u8; 1024];
    let bytes_read = file.read(&mut buffer)?;
    buffer.truncate(bytes_read);

    // Convert to string
    Ok(String::from_utf8_lossy(&buffer).to_string())
}

fn write_log(fs: &dyn ViFileSystem, message: &str) -> Result<(), ViError> {
    // Open file (creates if not exists)
    let mut file = fs.open("/var/log/app.log", OpenMode::Write)?;

    // Seek to end (append mode)
    file.seek(SeekFrom::End(0))?;

    // Write message
    file.write(message.as_bytes())?;

    Ok(())
}
```

---

## Block Device API

### ViBlockDevice Trait

**Location**: `libs/api/src/block.rs`

**Purpose**: Abstract interface for block storage devices (disks, SSDs, ramdisks)

```rust
pub trait ViBlockDevice: Send + Sync {
    /// Read blocks from device.
    ///
    /// # Arguments
    /// * `block_id` - Starting block number
    /// * `buf` - Buffer to read into (must be block_size * num_blocks)
    ///
    /// # Returns
    /// * `Ok(())` - Success
    /// * `Err(ViError::IoError)` - Hardware error
    fn read_block(&mut self, block_id: usize, buf: &mut [u8]) -> ViResult<()>;

    /// Write blocks to device.
    ///
    /// # Arguments
    /// * `block_id` - Starting block number
    /// * `buf` - Data to write (must be block_size * num_blocks)
    ///
    /// # Returns
    /// * `Ok(())` - Success
    /// * `Err(ViError::IoError)` - Hardware error
    fn write_block(&mut self, block_id: usize, buf: &[u8]) -> ViResult<()>;

    /// Get block size in bytes (typically 512 or 4096).
    fn block_size(&self) -> usize;

    /// Get total number of blocks on device.
    fn num_blocks(&self) -> u64;

    /// Flush any cached writes to physical device.
    fn flush(&mut self) -> ViResult<()>;
}
```

### Usage Example

```rust
fn read_sector(device: &mut dyn ViBlockDevice, sector: usize) -> Result<Vec<u8>, ViError> {
    let block_size = device.block_size();
    let mut buffer = vec![0u8; block_size];

    device.read_block(sector, &mut buffer)?;

    Ok(buffer)
}

fn write_sector(device: &mut dyn ViBlockDevice, sector: usize, data: &[u8]) -> Result<(), ViError> {
    assert_eq!(data.len(), device.block_size());

    device.write_block(sector, data)?;
    device.flush()?;  // Ensure data is written to physical media

    Ok(())
}
```

---

## Driver API

### ViDriver Trait

**Location**: `libs/api/src/driver.rs`

**Purpose**: Generic driver interface for hardware devices

```rust
pub trait ViDriver: Send + Sync {
    /// Initialize the driver.
    fn init(&mut self) -> ViResult<()>;

    /// Get driver name (for debugging).
    fn name(&self) -> &str;

    /// Handle interrupt (if this driver handles interrupts).
    fn handle_interrupt(&mut self) -> ViResult<()> {
        Err(ViError::NotSupported)
    }

    /// Shutdown driver and release resources.
    fn shutdown(&mut self) -> ViResult<()> {
        Ok(())
    }
}
```

### Specialized Driver Traits

**UART Driver** (`hal/traits/uart/`):
```rust
pub trait ViUART: Send + Sync {
    /// Write a single character.
    fn putc(&self, c: u8);

    /// Read a single character (non-blocking).
    /// Returns None if no character available.
    fn getc(&self) -> Option<u8>;

    /// Write a string.
    fn puts(&self, s: &str) {
        for byte in s.bytes() {
            self.putc(byte);
        }
    }
}
```

**Display Driver** (`hal/traits/display/`):
```rust
pub trait ViDisplay: Send + Sync {
    /// Get framebuffer information.
    fn info(&self) -> FramebufferInfo;

    /// Get mutable access to framebuffer.
    fn framebuffer(&mut self) -> &mut [u8];

    /// Flush framebuffer to screen (present).
    fn present(&mut self);
}

#[repr(C)]
pub struct FramebufferInfo {
    pub width: u32,
    pub height: u32,
    pub stride: u32,      // Bytes per row
    pub format: PixelFormat,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum PixelFormat {
    RGB888,    // 24-bit RGB
    RGBA8888,  // 32-bit RGBA
    BGR888,    // 24-bit BGR
}
```

---

## IPC Primitives

### Message Passing

**Send** (`syscall = 0`):
```rust
/// Send a message to another task (blocking).
///
/// # Arguments
/// * `target_id` - Task ID of recipient
/// * `message` - Message buffer (owned)
/// * `timeout_ms` - Timeout in milliseconds (0 = infinite)
///
/// # Returns
/// * `Ok(())` - Message delivered
/// * `Err(ViError::Timeout)` - Recipient did not receive within timeout
/// * `Err(ViError::InvalidTask)` - Target task does not exist
pub fn send(target_id: usize, message: Box<[u8]>, timeout_ms: usize) -> ViResult<()>;
```

**Receive** (`syscall = 1`):
```rust
/// Receive a message from any task (blocking).
///
/// # Arguments
/// * `buffer_size` - Maximum message size to receive
/// * `timeout_ms` - Timeout in milliseconds (0 = infinite)
///
/// # Returns
/// * `Ok((sender_id, message))` - Message received
/// * `Err(ViError::Timeout)` - No message within timeout
pub fn recv(buffer_size: usize, timeout_ms: usize) -> ViResult<(usize, Box<[u8]>)>;
```

**Call** (`syscall = 2`):
```rust
/// Synchronous RPC: Send message and wait for reply.
///
/// # Arguments
/// * `target_id` - Service task ID
/// * `request` - Request message
/// * `timeout_ms` - Timeout for entire RPC
///
/// # Returns
/// * `Ok(response)` - Reply from service
/// * `Err(ViError::Timeout)` - Service did not reply within timeout
pub fn call(target_id: usize, request: Box<[u8]>, timeout_ms: usize) -> ViResult<Box<[u8]>>;
```

**Reply** (`syscall = 3`):
```rust
/// Reply to a message sender (from service).
///
/// # Arguments
/// * `caller_id` - Task ID of caller (from recv)
/// * `response` - Response message
///
/// # Returns
/// * `Ok(())` - Reply delivered
/// * `Err(ViError::InvalidTask)` - Caller no longer waiting
pub fn reply(caller_id: usize, response: Box<[u8]>) -> ViResult<()>;
```

### Zero-Copy IPC

**Lease** (Temporary Borrow):
```rust
/// Lend a buffer to another task (caller retains ownership).
///
/// # Arguments
/// * `target_id` - Task to lend to
/// * `buffer` - Buffer to share
/// * `permissions` - READ, WRITE, or READ | WRITE
/// * `duration_ms` - Lease duration (0 = until revoked)
///
/// # Returns
/// * `Ok(lease_id)` - Lease handle
pub fn lend(
    target_id: usize,
    buffer: &mut [u8],
    permissions: LeasePermissions,
    duration_ms: usize
) -> ViResult<usize>;

/// Revoke a lease early.
pub fn revoke_lease(lease_id: usize) -> ViResult<()>;
```

**Grant** (Ownership Transfer):
```rust
/// Grant a buffer to another task (ownership transferred).
///
/// # Arguments
/// * `target_id` - Task to grant to
/// * `buffer` - Buffer to transfer (caller loses access)
///
/// # Returns
/// * `Ok(())` - Grant successful
/// * `Err(ViError::InvalidTask)` - Target task does not exist
///
/// # Safety
/// Caller must not access `buffer` after this call.
pub fn grant(target_id: usize, buffer: Box<[u8]>) -> ViResult<()>;

/// Receive a grant from another task.
///
/// # Returns
/// * `Ok((sender_id, buffer))` - Grant received
/// * `Err(ViError::NoGrant)` - No grant available
pub fn receive_grant() -> ViResult<(usize, Box<[u8]>)>;
```

---

## Task Management

### Task Creation

**Spawn** (`syscall = 5`):
```rust
/// Spawn a new task from an ELF binary.
///
/// # Arguments
/// * `cell_name` - Name of cell to load (e.g., "shell")
/// * `args` - Command-line arguments
///
/// # Returns
/// * `Ok(task_id)` - New task ID
/// * `Err(ViError::NotFound)` - Cell not found
/// * `Err(ViError::OutOfMemory)` - Cannot allocate stack/heap
pub fn spawn(cell_name: &str, args: &[&str]) -> ViResult<usize>;
```

**Exec** (`syscall = 6`):
```rust
/// Replace current task with a new Cell (like Unix exec).
///
/// # Arguments
/// * `cell_name` - Cell to execute
/// * `args` - Arguments
///
/// # Returns
/// * Does not return on success
/// * `Err(ViError::NotFound)` - Cell not found
pub fn exec(cell_name: &str, args: &[&str]) -> ViResult<!>;
```

**Exit** (`syscall = 60`):
```rust
/// Terminate current task.
///
/// # Arguments
/// * `exit_code` - Exit status (0 = success, non-zero = error)
///
/// # Returns
/// * Does not return
pub fn exit(exit_code: i32) -> !;
```

**Yield** (`syscall = 104`):
```rust
/// Voluntarily give up CPU to scheduler.
///
/// # Returns
/// * Returns when task is scheduled again
pub fn yield_now();
```

---

## Error Handling

### ViError Enum

**Location**: `libs/types/src/lib.rs`

```rust
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViError {
    // Generic errors
    InvalidArgument,
    OutOfMemory,
    NotSupported,

    // IPC errors
    Timeout,
    InvalidTask,
    NoGrant,

    // Filesystem errors
    NotFound,
    AlreadyExists,
    NotEmpty,
    IoError,
    PermissionDenied,

    // Hardware errors
    DeviceError,
    BusError,
}

pub type ViResult<T> = Result<T, ViError>;
```

### Error Handling Patterns

**Pattern 1: Propagate with `?`**:
```rust
fn open_and_read(fs: &dyn ViFileSystem, path: &str) -> ViResult<Vec<u8>> {
    let mut file = fs.open(path, OpenMode::Read)?;  // Propagate error
    let mut buffer = Vec::new();
    file.read(&mut buffer)?;  // Propagate error
    Ok(buffer)
}
```

**Pattern 2: Match and Handle**:
```rust
fn safe_open(fs: &dyn ViFileSystem, path: &str) -> Box<dyn ViFile> {
    match fs.open(path, OpenMode::Read) {
        Ok(file) => file,
        Err(ViError::NotFound) => {
            log::warn!("File not found, using default");
            create_default_file(fs, path).unwrap()
        }
        Err(e) => panic!("Cannot open file: {:?}", e),
    }
}
```

**Pattern 3: Unwrap for Critical Errors**:
```rust
fn must_open(fs: &dyn ViFileSystem, path: &str) -> Box<dyn ViFile> {
    fs.open(path, OpenMode::Read)
        .expect("Critical file missing, cannot continue")
}
```

---

## Usage Examples

### Example 1: File I/O Service

```rust
use api::fs::{ViFileSystem, ViFile, OpenMode};

pub struct FileService {
    fs: Arc<dyn ViFileSystem + Send + Sync>,
}

impl FileService {
    pub fn read_file(&self, path: &str) -> ViResult<Box<[u8]>> {
        // Open file
        let mut file = self.fs.open(path, OpenMode::Read)?;

        // Read all contents
        let mut contents = Vec::new();
        let mut buffer = [0u8; 4096];

        loop {
            let bytes_read = file.read(&mut buffer)?;
            if bytes_read == 0 { break; }  // EOF
            contents.extend_from_slice(&buffer[..bytes_read]);
        }

        Ok(contents.into_boxed_slice())
    }

    pub fn write_file(&self, path: &str, data: &[u8]) -> ViResult<()> {
        let mut file = self.fs.open(path, OpenMode::Write)?;

        let mut offset = 0;
        while offset < data.len() {
            let written = file.write(&data[offset..])?;
            offset += written;
        }

        Ok(())
    }

    pub fn list_directory(&self, path: &str) -> ViResult<Vec<DirEntry>> {
        let mut dir = self.fs.open(path, OpenMode::Read)?;
        let mut entries = Vec::new();

        while let Some(entry) = dir.read_dir()? {
            entries.push(entry);
        }

        Ok(entries)
    }
}
```

### Example 2: RPC Service

```rust
use ostd::syscall::{recv, reply};

pub fn rpc_server_loop() -> ! {
    loop {
        // Wait for incoming request
        let (caller_id, request) = recv(4096, 0).unwrap();

        // Parse request
        let response = match handle_request(&request) {
            Ok(data) => data,
            Err(e) => {
                log::error!("Request failed: {:?}", e);
                format!("ERROR: {:?}", e).into_bytes().into_boxed_slice()
            }
        };

        // Send response
        reply(caller_id, response).unwrap();
    }
}

fn handle_request(request: &[u8]) -> ViResult<Box<[u8]>> {
    // Deserialize request, process, serialize response
    let command = core::str::from_utf8(request)
        .map_err(|_| ViError::InvalidArgument)?;

    match command {
        "ping" => Ok(b"pong".to_vec().into_boxed_slice()),
        "version" => Ok(b"ViOS 0.2.0".to_vec().into_boxed_slice()),
        _ => Err(ViError::NotSupported),
    }
}
```

### Example 3: Zero-Copy Buffer Sharing

```rust
use ostd::syscall::{lend, grant, send, recv};

fn producer_task() {
    // Allocate large buffer
    let mut buffer = vec![0u8; 1_000_000].into_boxed_slice();

    // Fill buffer
    for (i, byte) in buffer.iter_mut().enumerate() {
        *byte = (i % 256) as u8;
    }

    // Option 1: Lend (temporary share, keep ownership)
    let lease_id = lend(consumer_task_id, &mut buffer, READ, 1000).unwrap();
    // Can still access buffer after lease expires

    // Option 2: Grant (permanent transfer, lose ownership)
    grant(consumer_task_id, buffer).unwrap();
    // Cannot access buffer anymore
}

fn consumer_task() {
    // Receive grant
    let (sender_id, buffer) = receive_grant().unwrap();

    // Process buffer (now owns it)
    process_data(&buffer);

    // No need to return buffer, ownership transferred
}
```

---

## API Evolution and Versioning

### Semantic Versioning

**API Stability Promise**:
- **Major version** (0.x → 1.x): Breaking changes allowed
- **Minor version** (0.2.x → 0.3.x): New features, backwards compatible
- **Patch version** (0.2.0 → 0.2.1): Bug fixes only

### Deprecation Policy

**Process**:
1. Mark API as `#[deprecated]` with reason and alternative
2. Keep deprecated API for at least one minor version
3. Remove in next major version

**Example**:
```rust
#[deprecated(since = "0.3.0", note = "Use `open_v2` instead")]
pub fn open(&self, path: &str) -> ViResult<Box<dyn ViFile>>;

pub fn open_v2(&self, path: &str, mode: OpenMode) -> ViResult<Box<dyn ViFile>>;
```

---

## Related Documentation

- [Architecture](./ARCHITECTURE.md) - System design and components
- [Coding Guide](./CODING_GUIDE.md) - How to implement APIs
- [Services Documentation](./SERVICES.md) - System services using these APIs

---

**Last Updated**: 2026-01-07
**Version**: 0.2.0
**Maintainer**: ViOS Team
