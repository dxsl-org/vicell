use alloc::collections::{BTreeMap, VecDeque};
use alloc::boxed::Box;
use log::{info, warn, error};
use super::tcb::{Task, TaskState};
use alloc::vec::Vec;
use types::*;
// use alloc::collections::BTreeMap;
// use alloc::collections::VecDeque;
// use log::{info, warn, error};

/// Round-Robin Scheduler with Central Task Table (Hubris-like)
pub struct Scheduler {
    pub tasks: BTreeMap<usize, Box<Task>>,
    pub zombies: Vec<Box<Task>>,
    pub ready_queue: VecDeque<usize>,
    pub current_task_id: Option<usize>,
    pub next_task_id: usize,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            tasks: BTreeMap::new(),
            zombies: Vec::new(),
            ready_queue: VecDeque::new(),
            current_task_id: None,
            next_task_id: 1,
        }
    }

    pub fn spawn(&mut self, name: &str, cell_id: CellId, allowed_drivers: alloc::vec::Vec<usize>) -> usize {
        let mut task = Box::new(Task::new(self.next_task_id, cell_id, name, allowed_drivers));
        task.state = TaskState::Ready;
        let id = task.id;
        
        // Stack Size: 8 Frames (32KB)
        use crate::task::STACK_PAGES as STACK_FRAMES; 

        // Allocate Kernel Stack
        let kstack = crate::task::stack::Stack::new_kernel(STACK_FRAMES).expect("OOM Stack");
        
        // Stack grows DOWN. Top is at end of region.
        let stack_top = kstack.top;
        let stack_base = kstack.base;
        
        // Zero the stack
        // SAFETY: We own the allocated stack memory.
        unsafe {
             core::ptr::write_bytes(stack_base as *mut u8, 0, STACK_FRAMES * crate::memory::paging::PAGE_SIZE);
        }
                
             let entry = task_entry_point as *const () as usize;
             let (gp, tp) = crate::task::get_kernel_gp_tp();
                
             task.context.sp = stack_top;
             task.context.ra = entry;
             task.trap_frame.sepc = entry;
             task.trap_frame.sstatus = 0x120;
             task.context.gp = gp;
             task.context.tp = tp;
             task.kernel_stack = Some(kstack);
                
             info!("Task '{}' (ID {}): Stack 0x{:X}-0x{:X}", name, id, stack_base, stack_top);
        
        self.tasks.insert(id, task);
        self.ready_queue.push_back(id);
        self.next_task_id += 1;
        id
    }

    pub fn spawn_thread(&mut self, name: &str, cell_id: CellId, allowed_drivers: alloc::vec::Vec<usize>, entry: usize, arg: usize) -> usize {
        let mut task = Box::new(Task::new(self.next_task_id, cell_id, name, allowed_drivers));
        task.state = TaskState::Ready;
        let id = task.id;
        
        use crate::task::STACK_PAGES as STACK_FRAMES;
        
        // Allocate Kernel Stack
        let kstack = crate::task::stack::Stack::new_kernel(STACK_FRAMES).expect("OOM Stack");
        
        let stack_top = kstack.top;
        let stack_base = kstack.base;
        
        // SAFETY: We own the allocated stack memory exclusively. The pointer is valid.
        // Setting up task context with valid register values for thread initialization.
        unsafe {
             core::ptr::write_bytes(stack_base as *mut u8, 0, STACK_FRAMES * crate::memory::paging::PAGE_SIZE);
             
             let (gp, tp) = crate::task::get_kernel_gp_tp();
             let trampoline = crate::hal::arch::thread_trampoline as usize;

             task.context.sp = stack_top;
             task.context.ra = trampoline;
             task.context.s0 = arg;
             task.context.s1 = entry;
             task.context.gp = gp;
             task.context.tp = tp;
             task.trap_frame.sepc = trampoline;
             task.trap_frame.sstatus = 0x120;
             task.kernel_stack = Some(kstack);
                
             info!("Thread '{}' (ID {}): Stack 0x{:X}-0x{:X}, Entry 0x{:X}, Arg 0x{:X}", 
                 name, id, stack_base, stack_top, entry, arg);
        }
        
        self.tasks.insert(id, task);
        self.ready_queue.push_back(id);
        self.next_task_id += 1;
        id
    }

    pub fn exit_task(&mut self, tid: usize) {
        info!("Task {} exiting...", tid);
        if let Some(task) = self.tasks.remove(&tid) {
            self.zombies.push(task);
        }
        
        // Remove from ready queue if present
        let mut new_kq = VecDeque::new();
        while let Some(id) = self.ready_queue.pop_front() {
            if id != tid { new_kq.push_back(id); }
        }
        self.ready_queue = new_kq;
    }

    /// Picks the next task to run and returns pointers for context switch.
    /// Returns: Option<(current_context_ptr, next_context_ptr)>
    pub fn pick_next(&mut self) -> Option<(*mut crate::hal::arch::Context, *const crate::hal::arch::Context)> {
        let now = crate::task::system_ticks();

        // 1. Wake up sleeping tasks
        let mut waking_tasks = VecDeque::new();
        for (id, task) in self.tasks.iter_mut() {
            let mut should_wake = false;
            if let TaskState::Sleeping { until } = &task.state {
                if now >= *until {
                    should_wake = true;
                }
            }
            if should_wake {
                task.state = TaskState::Ready;
                waking_tasks.push_back(*id);
            }
        }
        for id in waking_tasks {
            self.ready_queue.push_back(id);
        }

        // 2. Decide if current task needs to yield
        let current_id = self.current_task_id;
        if let Some(cid) = current_id {
            if let Some(task) = self.tasks.get_mut(&cid) {
                if task.state == TaskState::Running {
                    task.state = TaskState::Ready;
                    self.ready_queue.push_back(cid);
                }
            }
        }

        // 3. Get next task
        let next_id = self.ready_queue.pop_front();
        
        if let Some(nid) = next_id {
            if let Some(next_task) = self.tasks.get_mut(&nid) {
                next_task.state = TaskState::Running;
            }

            if Some(nid) == current_id {
                self.current_task_id = Some(nid);
                return None; // No switch needed
            }

            // SAFETY: We're creating raw pointers to task contexts to avoid borrow checker issues
            // during context switching.
            unsafe {
                let tasks_map_ptr = &mut self.tasks as *mut BTreeMap<usize, Box<Task>>;
                let next_ctx = (*tasks_map_ptr).get_mut(&nid).map(|t| &t.context as *const _);
                self.current_task_id = Some(nid);

                if let Some(cid) = current_id {
                    // Try to finding current in tasks or zombies
                    let curr_ctx = if let Some(t) = (*tasks_map_ptr).get_mut(&cid) {
                        Some(&mut t.context as *mut _)
                    } else {
                         self.zombies.iter_mut().find(|t| t.id == cid).map(|t| &mut t.context as *mut _)
                    };

                    if let (Some(c), Some(n)) = (curr_ctx, next_ctx) {
                        return Some((c, n));
                    }
                } else {
                    // First switch
                    if let Some(n) = next_ctx {
                        return Some((core::ptr::null_mut(), n));
                    }
                }
            }
        } else {
            // No ready tasks. 
            // If we are currently running a zombie (exiting), we MUST switch to something. (Boot Context)
             if let Some(cid) = current_id {
                 // Check if current is zombie
                 let is_zombie = self.zombies.iter().any(|t| t.id == cid);
                 if is_zombie {
                      unsafe {
                          let curr_ctx = self.zombies.iter_mut().find(|t| t.id == cid).map(|t| &mut t.context as *mut _);
                          if let Some(c) = curr_ctx {
                              // Switch to NULL next (Boot Context)
                              self.current_task_id = None;
                              return Some((c, core::ptr::null()));
                          }
                      }
                 }
             }
            
            self.current_task_id = None;
        }

        None
    }

    pub fn current_task_mut(&mut self) -> Option<&mut Task> {
        self.current_task_id.and_then(|id| self.tasks.get_mut(&id).map(|b| &mut **b))
    }
    
    pub fn current_task_ref(&self) -> Option<&Task> {
        self.current_task_id.and_then(|id| self.tasks.get(&id).map(|b| &**b))
    }
    
    pub fn has_ready_tasks(&self) -> bool {
        !self.ready_queue.is_empty()
    }
}

/// Default entry point for kernel tasks
#[no_mangle]
extern "C" fn task_entry_point() {
    // SAFETY: This is the entry point for new tasks. We need to:
    // 1. Force unlock the scheduler (safe because we're in a new task context)
    // 2. Initialize HAL for this task context
    // 3. Enable interrupts (safe because stack is properly set up)
    unsafe {
         crate::task::SCHEDULER.force_unlock();
         crate::hal::arch::init();
         // Enable Interrupts MANUALLY now that we're safe and stack is clean
         crate::hal::arch::enable_interrupts();
    }
    info!("Task started!");
    loop {
        for _ in 0..10_000_000 { core::hint::spin_loop(); }
        info!("Task tick (ID: {})...", crate::task::current_task_id());
        crate::task::yield_cpu();
    }
}
