# Welcome to ViOS Development!

> **Your guide to becoming a productive ViOS contributor**

## Table of Contents

1. [Welcome!](#welcome)
2. [What is ViOS?](#what-is-vios)
3. [Getting Started](#getting-started)
4. [Understanding the Codebase](#understanding-the-codebase)
5. [Your First Contribution](#your-first-contribution)
6. [Development Workflow](#development-workflow)
7. [Learning Resources](#learning-resources)
8. [Getting Help](#getting-help)
9. [Next Steps](#next-steps)

---

## Welcome!

Welcome to the ViOS project! We're excited to have you here. ViOS is not your typical operating system - it's a revolutionary approach to OS design that challenges decades of assumptions about how operating systems should work.

### What Makes ViOS Different?

**Traditional OS** (Linux, Windows, macOS):
- Process-based isolation using hardware MMU
- Separate address spaces require context switches
- IPC requires copying data between address spaces
- Large, monolithic kernel

**ViOS** (Cellular Single Address Space):
- Cell-based isolation using Rust's type system
- Single address space shared by all code
- Zero-copy IPC via ownership transfer
- Nano kernel (~7000 lines) with most functionality in Cells

### Who Should Contribute?

You're in the right place if you:
- ✅ Know Rust (or eager to learn deeply)
- ✅ Interested in operating systems
- ✅ Want to explore alternative OS architectures
- ✅ Like working on cutting-edge, research-y projects
- ✅ Appreciate safety and correctness

You don't need to be an expert! We welcome:
- First-time OS contributors
- Students learning systems programming
- Experienced developers from other domains
- Researchers exploring new ideas

---

## What is ViOS?

### The 30-Second Pitch

ViOS is a **Cellular Single Address Space (SAS) Operating System** built in Rust that uses **Language-Based Isolation (LBI)** instead of hardware memory protection. It's designed for the Edge-to-Cloud era, running on everything from tiny microcontrollers to cloud servers.

### Key Concepts

#### 1. Cellular Architecture

Software is organized as **Cells** (not processes):
- Each Cell is an independently compiled unit (.o file)
- Cells can be Native Rust, WASM, or sandboxed C/C++
- Cells are loaded and linked at runtime by the kernel

```
Traditional:          ViOS:
Process 1            Cell: shell
Process 2            Cell: vfs (service)
Process 3            Cell: disk (driver)
```

#### 2. Single Address Space (SAS)

All Cells share one virtual address space:
- No address space switches during IPC
- Direct pointer sharing (within safety rules)
- Zero-copy data transfer

```
Traditional:                   ViOS:
┌─────────────┐               ┌─────────────────┐
│ Process A   │               │  Single Address │
│ 0x00000000  │               │  Space          │
│    ...      │               │                 │
└─────────────┘               │  Kernel         │
┌─────────────┐               │  Cell A         │
│ Process B   │               │  Cell B         │
│ 0x00000000  │               │  Cell C         │
│    ...      │               │  ...            │
└─────────────┘               └─────────────────┘
```

#### 3. Language-Based Isolation (LBI)

Safety comes from Rust, not hardware:
- Cells compiled with `#![forbid(unsafe_code)]`
- Borrow checker prevents memory safety bugs
- Type system enforces API contracts
- No need for hardware page tables between Cells

#### 4. Zero-Copy IPC

Data transfer via ownership (like Rust's `move`):
- **Lease**: Temporary borrow (like `&T` or `&mut T`)
- **Grant**: Permanent transfer (like Rust `move`)
- No copying, no serialization overhead

### Architecture Overview

```
┌─────────────────────────────────────┐
│        Cells (User Space)           │
│  ┌──────┐  ┌──────┐  ┌──────┐     │
│  │ Apps │  │Services│ │Drivers│     │
│  └──────┘  └──────┘  └──────┘     │
├─────────────────────────────────────┤
│        Nano Kernel (7K LOC)         │
│  • ELF Loader & Linker              │
│  • Task Scheduler                   │
│  • Memory Manager                   │
│  • IPC Primitives                   │
├─────────────────────────────────────┤
│   Hardware Abstraction Layer (HAL)  │
│   RISC-V │  ARM  │  x86             │
└─────────────────────────────────────┘
```

---

## Getting Started

### Prerequisites

Before diving in, ensure you have:

1. **Rust Knowledge**: Intermediate level
   - Ownership, borrowing, lifetimes
   - Traits and trait objects
   - `no_std` environment basics
   - async/await (helpful but not required)

2. **Systems Knowledge**: Basic understanding
   - How CPUs work (registers, instructions)
   - Virtual memory concepts
   - What an operating system does

3. **Tools**: Installed and working
   - Rust nightly toolchain
   - QEMU for RISC-V
   - Git
   - Python 3

### Step 1: Installation

Follow the complete installation guide:

**👉 [Read INSTALLATION.md](./INSTALLATION.md)**

**Quick version**:
```bash
# Install Rust nightly
rustup default nightly
rustup component add rust-src rustfmt clippy
rustup target add riscv64gc-unknown-none-elf

# Clone repo
git clone https://github.com/your-org/vios.git
cd vios

# Install QEMU (Linux)
sudo apt install qemu-system-riscv64

# Build and run
cargo build --release
python3 create_ramdisk.py
./run.ps1  # or manual qemu command
```

### Step 2: Verify Setup

Run the kernel and verify you see:

```
VIOS K MAIN ENTRY
[INFO] Kernel started (Hart: 0, DTB: 0x87000000)
[INFO] Frame allocator initialized
[INFO] Paging initialized
[INFO] Heap initialized
[INFO] Scheduler initialized
ViOS Shell v0.2.0
vios>
```

**Success!** You're running ViOS. Type `help` at the prompt to see available commands.

---

## Understanding the Codebase

### The 20-Minute Tour

#### Phase 1: Read the Constitution (5 minutes)

**MUST READ**: `docs/agent.md` (or quick version: `CLAUDE.md` in root)

This file contains the prime directives and coding laws. It's short but essential.

Key takeaways:
- Cellular SAS philosophy
- Owned Buffers Rule for async
- Multi-arch from day 1
- Modern module style (no `mod.rs`)

#### Phase 2: Explore the Structure (5 minutes)

```bash
# Look at project structure
tree -L 2

# Key directories:
ls kernel/src/     # Nano kernel core
ls hal/            # Hardware abstraction
ls libs/           # Core types and APIs
ls cells/          # Applications, services, drivers
```

#### Phase 3: Read Core Documentation (10 minutes)

1. **[ARCHITECTURE.md](./ARCHITECTURE.md)** - Skim the overview section
   - Understand Cellular SAS concept
   - Look at the component diagram
   - Read about IPC (Lease/Grant)

2. **[CODING_GUIDE.md](./CODING_GUIDE.md)** - Scan the Golden Rules
   - Note the 8 core rules
   - Bookmark for later reference

### Deep Dive: Follow a Syscall

Let's trace a syscall from Cell to Kernel:

**1. Cell makes syscall** (`cells/apps/shell/`):
```rust
let mut file = ostd::fs::open("/test.txt", OpenMode::Read)?;
```

**2. User-space wrapper** (`libs/ostd/src/fs.rs`):
```rust
pub fn open(path: &str, mode: OpenMode) -> ViResult<Box<dyn ViFile>> {
    let result = unsafe {
        syscall::syscall3(
            ViSyscall::Open as usize,
            path.as_ptr() as usize,
            path.len(),
            mode as usize
        )
    };
    // Convert result to file handle
}
```

**3. Syscall trap** (`hal/arch/riscv/src/rv64/trap.rs`):
```rust
fn trap_handler(trap_frame: &mut ViTrapFrame) {
    match trap_frame.scause {
        8 | 9 | 11 => syscall_handler(trap_frame),  // Syscall
        // ...
    }
}
```

**4. Kernel handler** (`kernel/src/task/syscall.rs`):
```rust
pub fn handle_syscall(id: usize, args: &[usize]) -> isize {
    match ViSyscall::from(id) {
        ViSyscall::Open => sys_open(args),
        // ...
    }
}
```

**5. Filesystem implementation** (`kernel/src/fs/fat.rs`):
```rust
impl ViFileSystem for FatFs {
    fn open(&self, path: &str, mode: OpenMode) -> ViResult<Box<dyn ViFile>> {
        // Open file in FAT32 filesystem
    }
}
```

### Important Files to Know

| File | Purpose |
|------|---------|
| `kernel/src/main.rs` | Kernel entry point (kmain) |
| `kernel/src/task/tcb.rs` | Task Control Block definition |
| `kernel/src/task/scheduler.rs` | Task scheduler |
| `kernel/src/memory/frame.rs` | Physical memory allocator |
| `kernel/src/loader/elf.rs` | ELF loader for Cells |
| `libs/types/src/lib.rs` | Core types (VAddr, ViError, etc.) |
| `libs/api/src/fs.rs` | Filesystem trait definitions |
| `hal/arch/riscv/src/rv64/` | RISC-V 64-bit implementation |

---

## Your First Contribution

### Good First Issues

Look for issues tagged with `good-first-issue` or start here:

#### Easy (1-2 hours)
- [ ] Add a new syscall log message
- [ ] Fix a typo in documentation
- [ ] Add a test case for existing functionality
- [ ] Improve error message clarity

#### Medium (1 day)
- [ ] Implement a new shell command
- [ ] Add a new Cell application
- [ ] Improve logging in a subsystem
- [ ] Add documentation for an undocumented feature

#### Challenging (3-5 days)
- [ ] Implement a new HAL trait for a peripheral
- [ ] Add support for a new filesystem
- [ ] Optimize scheduler performance
- [ ] Add a new driver Cell

### Your First PR: Add a Shell Command

Let's add a simple `uptime` command to the shell.

**Step 1: Create a branch**
```bash
git checkout -b feature/shell-uptime-command
```

**Step 2: Find the shell code**
```bash
# Shell is a Cell application
cd cells/apps/shell/src
ls
# You'll see main.rs or similar
```

**Step 3: Read existing commands**
```rust
// Look for command handling pattern
match command {
    "help" => show_help(),
    "ls" => list_directory(),
    "cat" => cat_file(args),
    // Add your command here
}
```

**Step 4: Implement uptime**
```rust
"uptime" => {
    // Get system uptime from kernel
    let ticks = ostd::syscall::get_system_ticks();
    let seconds = ticks / 1000;  // Assuming 1000Hz timer
    ostd::println!("Uptime: {} seconds", seconds);
}
```

**Step 5: Test**
```bash
# Build and run
cargo build --release
python3 create_ramdisk.py
./run.ps1

# In ViOS shell:
vios> uptime
Uptime: 42 seconds
```

**Step 6: Create PR**
```bash
git add cells/apps/shell/
git commit -m "feat(shell): add uptime command

Shows system uptime in seconds.

Resolves #123"

git push origin feature/shell-uptime-command

# Then create PR on GitHub
```

---

## Development Workflow

### Daily Workflow

```
┌─────────────────────────────────────┐
│  1. Pull latest changes             │
│     git pull origin main            │
├─────────────────────────────────────┤
│  2. Create feature branch           │
│     git checkout -b feature/my-work │
├─────────────────────────────────────┤
│  3. Make changes                    │
│     vim kernel/src/...              │
├─────────────────────────────────────┤
│  4. Check and format                │
│     cargo check                     │
│     cargo fmt --all                 │
│     cargo clippy                    │
├─────────────────────────────────────┤
│  5. Build and test                  │
│     cargo build --release           │
│     ./run.ps1                       │
├─────────────────────────────────────┤
│  6. Commit changes                  │
│     git commit -m "feat: ..."      │
├─────────────────────────────────────┤
│  7. Push and create PR              │
│     git push origin feature/...     │
└─────────────────────────────────────┘
```

### Commit Message Format

Follow conventional commits:

```
<type>(<scope>): <subject>

<body>

<footer>
```

**Types**:
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation only
- `refactor`: Code refactoring
- `test`: Adding tests
- `chore`: Maintenance

**Examples**:
```
feat(kernel): add VirtIO network driver support

Implements ViNetworkStack trait for VirtIO NIC.
Supports basic send/receive operations.

Closes #42

---

fix(scheduler): correct task priority calculation

The previous implementation could cause priority inversion
under certain conditions. This fix ensures proper ordering.

Fixes #156

---

docs(api): document ViFileSystem trait methods

Added detailed documentation for all methods including
usage examples and error conditions.
```

### Code Review Process

1. **Submit PR**: Include description, test results, checklist
2. **Automated checks**: CI runs formatting, linting, builds
3. **Human review**: Maintainer reviews code
4. **Address feedback**: Make requested changes
5. **Approval**: Maintainer approves
6. **Merge**: PR merged to main

**What reviewers look for**:
- ✅ Follows [CODING_GUIDE.md](./CODING_GUIDE.md)
- ✅ No unsafe code in Cells
- ✅ Proper error handling
- ✅ Tests for new features
- ✅ Documentation updated
- ✅ Builds without warnings

---

## Learning Resources

### Essential Reading

**Week 1**: Basics
1. [ARCHITECTURE.md](./ARCHITECTURE.md) - System design
2. [CODING_GUIDE.md](./CODING_GUIDE.md) - How to write code
3. `.codebase/01-core.md` - Cellular philosophy
4. `.codebase/02-memory.md` - SAS memory model

**Week 2**: Deeper Dive
1. [API.md](./API.md) - Complete API reference
2. `.codebase/03-runtime.md` - Async and safety
3. `.codebase/04-hardware.md` - Multi-arch HAL
4. Explore: `kernel/src/task/` and `kernel/src/memory/`

**Week 3**: Advanced Topics
1. `.codebase/05-application.md` - Cell types (Native/WASM)
2. `.codebase/06-graphics.md` - Compositor and display
3. `.codebase/07-networking.md` - Network stack
4. Explore: `hal/arch/riscv/` for low-level details

### External Resources

**Rust for OS Development**:
- [The Rust Programming Language Book](https://doc.rust-lang.org/book/)
- [Rust Embedded Book](https://rust-embedded.github.io/book/)
- [Writing an OS in Rust](https://os.phil-opp.com/)

**RISC-V**:
- [RISC-V Specifications](https://riscv.org/specifications/)
- [RISC-V Assembly Programmer's Manual](https://github.com/riscv-non-isa/riscv-asm-manual)

**OS Concepts**:
- [OSDev Wiki](https://wiki.osdev.org/)
- [Operating Systems: Three Easy Pieces](https://pages.cs.wisc.edu/~remzi/OSTEP/)

### Recommended Learning Path

**Path 1: Application Developer**
```
1. Learn Rust basics
2. Read CODING_GUIDE.md
3. Write a simple Cell application
4. Study libs/ostd/ API
5. Build something useful!
```

**Path 2: Kernel Developer**
```
1. Learn Rust + no_std
2. Read ARCHITECTURE.md thoroughly
3. Study kernel/src/task/
4. Understand memory management
5. Trace a syscall end-to-end
6. Make a small kernel improvement
```

**Path 3: HAL Developer**
```
1. Learn Rust + embedded
2. Read .codebase/04-hardware.md
3. Study existing HAL implementation
4. Learn target architecture (RISC-V/ARM/x86)
5. Implement new HAL trait or architecture
```

---

## Getting Help

### Where to Ask Questions

**GitHub Discussions**: General questions, design discussions
- Topic: "How does X work?"
- Topic: "Why was Y designed this way?"
- Topic: "Help with Z error"

**GitHub Issues**: Bug reports, feature requests
- "Kernel panics when..."
- "Add support for..."
- "Improve performance of..."

**Chat** (Discord/Matrix): Real-time help
- Quick questions
- Debugging assistance
- Coordination

### How to Ask Good Questions

**Good question template**:

```markdown
## What I'm trying to do
I want to add a new syscall for getting CPU frequency.

## What I've tried
1. Read ARCHITECTURE.md section on syscalls
2. Looked at existing syscall implementations
3. Added syscall number to ViSyscall enum

## Where I'm stuck
How do I get CPU frequency from the HAL? I see `hal::ARCH` but
I'm not sure which method to call.

## My environment
- ViOS version: main branch (commit abc123)
- Target: RISC-V 64-bit
- Host: Linux Ubuntu 22.04
```

**Bad question**:
> "It doesn't work. Help!"

### Debugging Tips

**Kernel won't boot**:
```bash
# Add debug output
# In kernel/src/main.rs
log::info!("Checkpoint 1");  // Add these throughout

# Check QEMU logs
qemu-system-riscv64 ... -d int,cpu 2> qemu.log
cat qemu.log
```

**Cell crashes**:
```rust
// Add panic message
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    ostd::println!("PANIC: {}", info);  // Will show panic location
    ostd::exit(1)
}
```

**Build errors**:
```bash
# Clean and rebuild
cargo clean
cargo build --release -v  # Verbose output

# Check Rust version
rustc --version  # Should be nightly

# Update everything
rustup update nightly
cargo update
```

---

## Next Steps

### Week 1 Goals

- [ ] Successfully build and run ViOS
- [ ] Read ARCHITECTURE.md overview
- [ ] Read CODING_GUIDE.md golden rules
- [ ] Trace one syscall through the code
- [ ] Join GitHub Discussions and introduce yourself

### Week 2 Goals

- [ ] Read all `.codebase/*.md` files
- [ ] Modify the shell to add a simple command
- [ ] Successfully submit your first PR
- [ ] Review someone else's PR

### Month 1 Goals

- [ ] Understand the Cellular SAS model deeply
- [ ] Be able to explain ViOS to someone else
- [ ] Contribute 3-5 PRs (docs, code, tests)
- [ ] Help answer questions from newer contributors

### Long-term Goals

- [ ] Become a domain expert (HAL, kernel, cells, etc.)
- [ ] Mentor new contributors
- [ ] Design and implement a major feature
- [ ] Present ViOS at a conference/meetup

---

## Community Guidelines

### Be Respectful

- Assume good intent
- Be patient with newcomers
- Give constructive feedback
- Celebrate others' contributions

### Be Collaborative

- Share knowledge freely
- Ask for help when stuck
- Help others when you can
- Document what you learn

### Be Curious

- Question assumptions (respectfully)
- Propose improvements
- Experiment and share results
- Learn from mistakes

---

## Recognition

Contributors are the heart of ViOS. We recognize contributions in:

- **CONTRIBUTORS.md**: All contributors listed
- **Release notes**: Major contributions highlighted
- **Commit log**: Your work is permanent history
- **Community**: Recognition in discussions and chats

Every contribution matters, whether it's:
- Fixing a typo
- Answering a question
- Reviewing a PR
- Writing documentation
- Adding a feature

---

## Quick Reference Card

Print and keep handy:

```
┌──────────────────────────────────────┐
│         ViOS Quick Reference         │
├──────────────────────────────────────┤
│ Build:      cargo build --release    │
│ Format:     cargo fmt --all          │
│ Lint:       cargo clippy             │
│ Run:        ./run.ps1                │
│ Clean:      cargo clean              │
├──────────────────────────────────────┤
│ Docs:                                │
│  • ARCHITECTURE.md - System design   │
│  • CODING_GUIDE.md - How to code     │
│  • API.md - API reference            │
│  • This file - Getting started       │
├──────────────────────────────────────┤
│ Golden Rules:                        │
│  1. Interface is sacred              │
│  2. Owned buffers for async          │
│  3. Multi-arch aware                 │
│  4. Cells: #![forbid(unsafe_code)]   │
│  5. No mod.rs                        │
│  6. Vi prefix for public traits      │
│  7. dyn Trait for polymorphism       │
│  8. Implement Drop for resources     │
├──────────────────────────────────────┤
│ Help: GitHub Discussions             │
│ Chat: Discord/Matrix                 │
│ Bugs: GitHub Issues                  │
└──────────────────────────────────────┘
```

---

## Congratulations!

You've completed the onboarding guide. You're now ready to contribute to ViOS!

**Remember**:
- Start small
- Ask questions
- Read documentation
- Have fun!

**Welcome to the team! 🎉**

---

**Last Updated**: 2026-01-07
**Version**: 0.2.0
**Maintainer**: ViOS Team

---

## Appendix: First Day Checklist

Print this and check off as you go:

- [ ] Rust nightly installed
- [ ] QEMU installed
- [ ] Repository cloned
- [ ] ViOS builds successfully
- [ ] ViOS runs in QEMU
- [ ] Read README.md
- [ ] Read ARCHITECTURE.md (at least overview)
- [ ] Read CODING_GUIDE.md (at least golden rules)
- [ ] Read .codebase/agent.md
- [ ] Explored kernel/src/ directory
- [ ] Explored cells/ directory
- [ ] Joined GitHub Discussions
- [ ] Introduced yourself to community
- [ ] Picked a "good first issue"

**Welcome aboard! 🚀**
