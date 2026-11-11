//! `#[global_allocator]` on the process heap (`fnd_mem_alloc`, 8-byte aligned
//! blocks). Larger alignments over-allocate and keep the block address in the
//! word before the returned pointer.

use core::alloc::{GlobalAlloc, Layout};

use crate::sys;

const BLOCK_ALIGN: usize = 8;

struct FndHeap;

unsafe impl GlobalAlloc for FndHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size().max(1);
        if layout.align() <= BLOCK_ALIGN {
            return unsafe { sys::fnd_mem_alloc(size as u32) };
        }
        let Some(total) = size.checked_add(layout.align()) else {
            return core::ptr::null_mut();
        };
        let block = unsafe { sys::fnd_mem_alloc(total as u32) };
        if block.is_null() {
            return block;
        }
        // At least one word below the aligned address belongs to the block.
        let address = (block as usize + layout.align()) & !(layout.align() - 1);
        let p = address as *mut u8;
        unsafe { (p as *mut *mut u8).sub(1).write(block) };
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        let block = if layout.align() <= BLOCK_ALIGN {
            p
        } else {
            unsafe { (p as *mut *mut u8).sub(1).read() }
        };
        unsafe { sys::fnd_mem_free(block) }
    }
}

#[global_allocator]
static HEAP: FndHeap = FndHeap;
