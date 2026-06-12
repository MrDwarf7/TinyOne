extern crate std;

use ralloc::RallocBuffer;

/// Allocate small, medium, and large buffers in interleaved order, free every
/// other one to punch holes of varying sizes, then reallocate into those holes.
/// Exercises coalesce paths when adjacent free blocks of varying sizes merge.
#[test]
fn mixed_size_interleave_does_not_exhaust_arena() {
    let sizes = [8usize, 64, 256, 16, 128, 32, 512, 24];

    for round in 0..16 {
        let mut handles: Vec<RallocBuffer> = sizes
            .iter()
            .enumerate()
            .map(|(index, &n)| {
                let mut buffer = RallocBuffer::new(n).expect("initial allocation should succeed");
                let marker = round as u8 ^ index as u8 ^ n as u8;
                buffer.as_mut_slice().fill(marker);
                assert!(buffer.as_slice().iter().all(|&byte| byte == marker));
                buffer
            })
            .collect();

        // Free every other handle, punching holes of varying sizes.
        let mut i = 0;
        handles.retain(|_| {
            let keep = i % 2 == 0;
            i += 1;
            keep
        });

        // Allocate into the freed holes; the arena must not fail.
        for (index, &n) in sizes.iter().enumerate().skip(1).step_by(2) {
            let mut buf = RallocBuffer::new(n).expect("re-allocation into freed holes should succeed");
            let marker = 0xa0u8 ^ round as u8 ^ index as u8;
            buf.as_mut_slice().fill(marker);
            assert!(buf.as_slice().iter().all(|&byte| byte == marker));
            handles.push(buf);
        }

        for buffer in &handles {
            assert_eq!(buffer.as_ptr().addr() % core::mem::align_of::<usize>(), 0);
            assert!(!buffer.as_slice().is_empty());
        }

        drop(handles);
    }
}

/// Allocate an over-aligned buffer, free it, then allocate a normal-alignment
/// buffer of the same size. Confirms that the leading padding introduced for
/// alignment is reclaimed and merged into the free pool rather than left as a
/// stranded fragment.
#[test]
fn over_aligned_free_reclaimed_by_normal_alloc() {
    const SIZE: usize = 128;
    const ALIGN: usize = 64;

    let mut aligned = RallocBuffer::try_new_aligned(SIZE, ALIGN).expect("over-aligned allocation should succeed");
    assert_eq!(aligned.as_ptr().addr() % ALIGN, 0);
    aligned.as_mut_slice().fill(0x5a);
    assert!(aligned.as_slice().iter().all(|&byte| byte == 0x5a));
    drop(aligned);

    // Must not fail — the freed region (including any leading padding) must
    // be available to satisfy a same-sized normal-alignment request.
    let mut normal = RallocBuffer::new(SIZE).expect("normal alloc after aligned free should succeed");
    normal.as_mut_slice().fill(0x33);
    assert!(normal.as_slice().iter().all(|&byte| byte == 0x33));
    drop(normal);

    // Confirm a second over-aligned alloc also succeeds — no stranded padding.
    let mut second_aligned =
        RallocBuffer::try_new_aligned(SIZE, ALIGN).expect("second over-aligned alloc should succeed");
    assert_eq!(second_aligned.as_ptr().addr() % ALIGN, 0);
    second_aligned.as_mut_slice().fill(0xc3);
    assert!(second_aligned.as_slice().iter().all(|&byte| byte == 0xc3));
    drop(second_aligned);

    for align in [16usize, 32, 64, 128] {
        let buffer =
            RallocBuffer::try_new_aligned(32, align).expect("arena should still satisfy aligned requests after frees");
        assert_eq!(buffer.as_ptr().addr() % align, 0);
    }
}

/// Allocate and free the same-sized block repeatedly. Fragmentation must not
/// accumulate: later cycles must not fail while earlier allocations have been
/// freed, and multiple simultaneous allocations must succeed afterwards.
#[test]
fn repeated_same_size_cycle_does_not_fragment() {
    const SIZE: usize = 64;
    const CYCLES: usize = 256;

    for i in 0..CYCLES {
        let mut buf = RallocBuffer::new(SIZE).unwrap_or_else(|| panic!("allocation should succeed on cycle {i}"));
        let marker = i as u8;
        buf.as_mut_slice().fill(marker);
        assert!(buf.as_slice().iter().all(|&byte| byte == marker));
        drop(buf);
    }

    // After all cycles, multiple simultaneous allocations must still succeed.
    let bufs: Vec<_> = (0..4)
        .map(|index| {
            let mut buffer = RallocBuffer::new(SIZE).expect("simultaneous alloc after cycles should succeed");
            buffer.as_mut_slice().fill(0xe0 | index);
            buffer
        })
        .collect();
    for (index, buffer) in bufs.iter().enumerate() {
        assert!(buffer.as_slice().iter().all(|&byte| byte == (0xe0 | index as u8)));
    }
    drop(bufs);
}
