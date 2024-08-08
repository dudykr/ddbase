use std::{
    mem,
    ptr::{self, NonNull},
};

use super::{capacity::Capacity, Repr};

pub struct HeapStr {
    ptr: ptr::NonNull<u8>,
    len: Capacity,
}

static_assertions::assert_eq_size!(HeapStr, Repr);

impl HeapStr {
    pub unsafe fn new(text: &str) -> Self {
        let len = Capacity::new(text.len());
        let ptr = NonNull::new_unchecked(text as *const str as *mut u8);
        Self { ptr, len }
    }

    pub fn len(&self) -> usize {
        unsafe { self.len.as_usize() }
    }

    pub fn as_str(&self) -> &str {
        unsafe {
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.ptr.as_ptr(), self.len()))
        }
    }

    #[inline]
    pub fn dealloc(&mut self) {
        deallocate_ptr(self.ptr, self.len)
    }
}

/// Deallocates a buffer on the heap, handling when the capacity is also stored
/// on the heap
#[inline]
pub fn deallocate_ptr(ptr: ptr::NonNull<u8>, cap: Capacity) {
    #[cold]
    fn deallocate_with_capacity_on_heap(ptr: ptr::NonNull<u8>) {
        // re-adjust the pointer to include the capacity that's on the heap
        let adj_ptr = ptr.as_ptr().wrapping_sub(mem::size_of::<usize>());
        // read the capacity from the heap so we know how much to deallocate
        let mut buf = [0u8; mem::size_of::<usize>()];
        // SAFETY: `src` and `dst` don't overlap, and are valid for usize number of
        // bytes
        unsafe {
            ptr::copy_nonoverlapping(adj_ptr, buf.as_mut_ptr(), mem::size_of::<usize>());
        }
        let capacity = usize::from_ne_bytes(buf);
        // SAFETY: We know the pointer is not null since we got it as a NonNull
        let ptr = unsafe { ptr::NonNull::new_unchecked(adj_ptr) };
        // SAFETY: We checked above that our capacity is on the heap, and we readjusted
        // the pointer to reference the capacity
        unsafe { heap_capacity::dealloc(ptr, capacity) }
    }

    if cap.is_heap() {
        deallocate_with_capacity_on_heap(ptr);
    } else {
        // SAFETY: Our capacity is always inline on 64-bit archs
        unsafe { inline_capacity::dealloc(ptr, cap.as_usize()) }
    }
}

mod heap_capacity {
    use core::ptr;
    use std::alloc;

    use super::HeapStr;

    #[inline]
    pub fn alloc(capacity: usize) -> ptr::NonNull<u8> {
        let layout = layout(capacity);
        debug_assert!(layout.size() > 0);

        // SAFETY: `alloc(...)` has undefined behavior if the layout is zero-sized. We
        // know the layout can't be zero-sized though because we're always at
        // least allocating one `usize`
        let raw_ptr = unsafe { alloc::alloc(layout) };

        // Check to make sure our pointer is non-null, some allocators return null
        // pointers instead of panicking
        match ptr::NonNull::new(raw_ptr) {
            Some(ptr) => ptr,
            None => alloc::handle_alloc_error(layout),
        }
    }

    /// Deallocates a pointer which references a `HeapBuffer` whose capacity is
    /// on the heap
    ///
    /// # Saftey
    /// * `ptr` must point to the start of a `HeapBuffer` whose capacity is on
    ///   the heap. i.e. we must have `ptr -> [cap<usize> ; string<bytes>]`
    pub unsafe fn dealloc(ptr: ptr::NonNull<u8>, capacity: usize) {
        let layout = layout(capacity);
        alloc::dealloc(ptr.as_ptr(), layout);
    }

    #[repr(C)]
    struct HeapBufferInnerHeapCapacity {
        capacity: usize,
        buffer: HeapStr,
    }

    #[inline(always)]
    pub fn layout(capacity: usize) -> alloc::Layout {
        let buffer_layout = alloc::Layout::array::<u8>(capacity).expect("valid capacity");
        alloc::Layout::new::<HeapBufferInnerHeapCapacity>()
            .extend(buffer_layout)
            .expect("valid layout")
            .0
            .pad_to_align()
    }
}

mod inline_capacity {
    use core::ptr;
    use std::alloc;

    use super::HeapStr;

    /// # SAFETY:
    /// * `capacity` must be > 0
    #[inline]
    pub unsafe fn alloc(capacity: usize) -> ptr::NonNull<u8> {
        let layout = layout(capacity);
        debug_assert!(layout.size() > 0);

        // SAFETY: `alloc(...)` has undefined behavior if the layout is zero-sized. We
        // specify that `capacity` must be > 0 as a constraint to uphold the
        // safety of this method. If capacity is greater than 0, then our layout
        // will be non-zero-sized.
        let raw_ptr = alloc::alloc(layout);

        // Check to make sure our pointer is non-null, some allocators return null
        // pointers instead of panicking
        match ptr::NonNull::new(raw_ptr) {
            Some(ptr) => ptr,
            None => alloc::handle_alloc_error(layout),
        }
    }

    /// Deallocates a pointer which references a `HeapBuffer` whose capacity is
    /// stored inline
    ///
    /// # Saftey
    /// * `ptr` must point to the start of a `HeapBuffer` whose capacity is on
    ///   the inline
    pub unsafe fn dealloc(ptr: ptr::NonNull<u8>, capacity: usize) {
        let layout = layout(capacity);
        alloc::dealloc(ptr.as_ptr(), layout);
    }

    #[repr(C)]
    struct HeapBufferInnerInlineCapacity {
        buffer: HeapStr,
    }

    #[inline(always)]
    pub fn layout(capacity: usize) -> alloc::Layout {
        let buffer_layout = alloc::Layout::array::<u8>(capacity).expect("valid capacity");
        alloc::Layout::new::<HeapBufferInnerInlineCapacity>()
            .extend(buffer_layout)
            .expect("valid layout")
            .0
            .pad_to_align()
    }
}
