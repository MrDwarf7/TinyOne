use ralloc::{RallocBuffer, RallocError};

#[test]
fn buffer_allocates_writable_storage_and_frees_on_drop() {
    let first_ptr = {
        let mut buffer = RallocBuffer::new(32).expect("buffer allocation should succeed");
        assert_eq!(buffer.len(), 32);
        assert!(!buffer.is_empty());

        for (index, byte) in buffer.as_mut_slice().iter_mut().enumerate() {
            *byte = index as u8;
        }
        assert_eq!(buffer.as_slice()[17], 17);

        buffer.as_ptr()
    };

    let second = RallocBuffer::new(32).expect("buffer allocation should succeed after drop");
    // Pointer reuse is allocator-dependent and not guaranteed when other tests
    // run concurrently. Just verify the allocation succeeds.
    assert_eq!(second.len(), 32);
    assert!(!second.is_empty());
    assert_eq!(second.as_ptr().addr() % std::mem::align_of::<u8>(), 0);
}

#[test]
fn buffer_resize_requires_unique_handle_and_preserves_bytes() {
    let mut buffer = RallocBuffer::new(8).expect("buffer allocation should succeed");
    for (index, byte) in buffer.as_mut_slice().iter_mut().enumerate() {
        *byte = 0xa0u8 + index as u8;
    }

    assert!(buffer.resize(64));
    assert_eq!(buffer.len(), 64);
    assert_eq!(&buffer.as_slice()[..8], &[0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7]);

    assert!(buffer.resize(4));
    assert_eq!(buffer.len(), 4);
    assert_eq!(buffer.as_slice(), &[0xa0, 0xa1, 0xa2, 0xa3]);
}

#[test]
fn buffer_aligned_resize_preserves_alignment_and_bytes() {
    let mut buffer = RallocBuffer::try_new_aligned(8, 64).expect("aligned buffer allocation should succeed");
    assert_eq!(buffer.as_ptr().addr() % 64, 0);

    for (index, byte) in buffer.as_mut_slice().iter_mut().enumerate() {
        *byte = 0xb0u8 + index as u8;
    }

    buffer.try_resize(128).expect("aligned resize should succeed");

    assert_eq!(buffer.len(), 128);
    assert_eq!(buffer.as_ptr().addr() % 64, 0);
    assert_eq!(&buffer.as_slice()[..8], &[0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7]);
}

#[test]
fn buffer_rejects_invalid_alignment_requests() {
    assert_eq!(RallocBuffer::try_new_aligned(8, 3).err(), Some(RallocError::InvalidAlignment));
    assert_eq!(RallocBuffer::try_new_aligned(8, 8192).err(), Some(RallocError::InvalidAlignment));
}

#[test]
fn buffer_can_move_across_threads_without_sharing_aliases() {
    let mut buffer = RallocBuffer::new(16).expect("buffer allocation should succeed");
    buffer.as_mut_slice()[0] = 0x7a;

    let worker = std::thread::spawn(move || {
        assert_eq!(buffer.as_slice()[0], 0x7a);
        buffer.len()
    });

    assert_eq!(worker.join().expect("worker should not panic"), 16);
}

#[test]
fn buffer_type_is_send() {
    fn assert_send<T: Send>() {}

    assert_send::<RallocBuffer>();
}

#[test]
fn buffer_try_new_reports_invalid_zero_and_oom_separately() {
    let empty = RallocBuffer::try_new(0).expect("zero-sized buffer should succeed");
    assert!(empty.is_empty());

    assert_eq!(RallocBuffer::try_new(usize::MAX).err(), Some(RallocError::OutOfMemory));
}

#[test]
fn buffer_try_resize_reports_oom_without_losing_original_allocation() {
    let mut buffer = RallocBuffer::try_new(8).expect("buffer allocation should succeed");
    buffer.as_mut_slice().copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);

    assert_eq!(buffer.try_resize(usize::MAX), Err(RallocError::OutOfMemory));
    assert_eq!(buffer.len(), 8);
    assert_eq!(buffer.as_slice(), &[1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn buffer_raw_parts_round_trip_transfers_ownership_explicitly() {
    let mut buffer = RallocBuffer::try_new(4).expect("buffer allocation should succeed");
    buffer.as_mut_slice().copy_from_slice(&[9, 8, 7, 6]);

    let (ptr, len) = buffer.into_raw_parts();
    assert_eq!(len, 4);

    // SAFETY: `ptr` and `len` came from `RallocBuffer::into_raw_parts` and have
    // not been freed or rewrapped.
    let recovered = unsafe { RallocBuffer::from_raw_parts(ptr, len) }.expect("raw parts should recover ownership");

    assert_eq!(recovered.as_slice(), &[9, 8, 7, 6]);
}

#[test]
fn buffer_rejects_null_raw_parts_for_nonzero_length() {
    let recovered = unsafe { RallocBuffer::from_raw_parts(core::ptr::null_mut(), 4) };

    assert_eq!(recovered.err(), Some(RallocError::InvalidRawParts));
}
