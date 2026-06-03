#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

/// Stable identifier for a block device known to the supervisor or a storage service.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct BlockDeviceId(u64);

impl BlockDeviceId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Size, in bytes, of one logical block.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct BlockSize(u32);

impl BlockSize {
    pub const MIN: u32 = 1;

    pub const fn new(bytes: u32) -> Result<Self, BlockError> {
        if bytes < Self::MIN {
            Err(BlockError::InvalidBlockSize)
        } else {
            Ok(Self(bytes))
        }
    }

    pub const fn bytes(self) -> u32 {
        self.0
    }

    pub const fn bytes_usize(self) -> usize {
        self.0 as usize
    }
}

/// Logical block address.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Lba(u64);

impl Lba {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Number of logical blocks in a device or request.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SectorCount(u64);

impl SectorCount {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Contiguous range of logical blocks.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct BlockRange {
    start: Lba,
    count: SectorCount,
}

impl BlockRange {
    pub const fn new(start: Lba, count: SectorCount) -> Self {
        Self { start, count }
    }

    pub const fn start(self) -> Lba {
        self.start
    }

    pub const fn count(self) -> SectorCount {
        self.count
    }

    pub fn end_exclusive(self) -> Result<Lba, BlockError> {
        if self.count.is_empty() {
            return Err(BlockError::EmptyRange);
        }

        self.start
            .get()
            .checked_add(self.count.get())
            .map(Lba::new)
            .ok_or(BlockError::RangeOverflow)
    }

    pub fn validate_within(self, device_sectors: SectorCount) -> Result<(), BlockError> {
        let end = self.end_exclusive()?;
        if end.get() > device_sectors.get() {
            Err(BlockError::OutOfBounds)
        } else {
            Ok(())
        }
    }

    pub fn byte_len(self, block_size: BlockSize) -> Result<usize, BlockError> {
        if self.count.is_empty() {
            return Err(BlockError::EmptyRange);
        }

        let sectors = usize::try_from(self.count.get()).map_err(|_| BlockError::RangeOverflow)?;
        sectors
            .checked_mul(block_size.bytes_usize())
            .ok_or(BlockError::RangeOverflow)
    }
}

/// Monotonic request identifier assigned by a queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct BlockRequestId(u64);

impl BlockRequestId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Kind-specific request payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlockRequestKind {
    Read(BlockRange),
    Write { range: BlockRange, data: Vec<u8> },
    Flush,
}

/// A queued block operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockRequest {
    id: BlockRequestId,
    device_id: BlockDeviceId,
    kind: BlockRequestKind,
}

impl BlockRequest {
    pub const fn new(id: BlockRequestId, device_id: BlockDeviceId, kind: BlockRequestKind) -> Self {
        Self {
            id,
            device_id,
            kind,
        }
    }

    pub const fn id(&self) -> BlockRequestId {
        self.id
    }

    pub const fn device_id(&self) -> BlockDeviceId {
        self.device_id
    }

    pub const fn kind(&self) -> &BlockRequestKind {
        &self.kind
    }

    pub fn into_kind(self) -> BlockRequestKind {
        self.kind
    }
}

/// Kind-specific response payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlockResponseKind {
    Read { data: Vec<u8> },
    Write { bytes: usize },
    Flush,
}

/// Successful completion data for a block request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockResponse {
    request_id: BlockRequestId,
    device_id: BlockDeviceId,
    kind: BlockResponseKind,
}

impl BlockResponse {
    pub const fn new(
        request_id: BlockRequestId,
        device_id: BlockDeviceId,
        kind: BlockResponseKind,
    ) -> Self {
        Self {
            request_id,
            device_id,
            kind,
        }
    }

    pub const fn request_id(&self) -> BlockRequestId {
        self.request_id
    }

    pub const fn device_id(&self) -> BlockDeviceId {
        self.device_id
    }

    pub const fn kind(&self) -> &BlockResponseKind {
        &self.kind
    }
}

/// Backend-independent block operation errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlockError {
    InvalidBlockSize,
    EmptyRange,
    RangeOverflow,
    OutOfBounds,
    BufferSizeMismatch,
    DeviceOffline,
    DeviceFaulted,
    ReadOnly,
    QueueEmpty,
    DeviceMismatch,
    Io,
}

/// Static properties advertised by a block device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockDeviceInfo {
    pub id: BlockDeviceId,
    pub block_size: BlockSize,
    pub sectors: SectorCount,
    pub read_only: bool,
    pub write_cache: bool,
}

impl BlockDeviceInfo {
    pub const fn new(
        id: BlockDeviceId,
        block_size: BlockSize,
        sectors: SectorCount,
        read_only: bool,
        write_cache: bool,
    ) -> Self {
        Self {
            id,
            block_size,
            sectors,
            read_only,
            write_cache,
        }
    }

    pub fn validate_range(self, range: BlockRange) -> Result<(), BlockError> {
        range.validate_within(self.sectors)
    }

    pub fn expected_buffer_len(self, range: BlockRange) -> Result<usize, BlockError> {
        self.validate_range(range)?;
        range.byte_len(self.block_size)
    }
}

/// Runtime availability state of a block device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockDeviceState {
    Online,
    Offline,
    Faulted,
}

impl BlockDeviceState {
    pub const fn ensure_available(self) -> Result<(), BlockError> {
        match self {
            Self::Online => Ok(()),
            Self::Offline => Err(BlockError::DeviceOffline),
            Self::Faulted => Err(BlockError::DeviceFaulted),
        }
    }
}

/// Storage-backend agnostic block device interface.
pub trait BlockDevice {
    fn info(&self) -> BlockDeviceInfo;

    fn state(&self) -> BlockDeviceState;

    fn read_blocks(&mut self, range: BlockRange, buffer: &mut [u8]) -> Result<(), BlockError>;

    fn write_blocks(&mut self, range: BlockRange, data: &[u8]) -> Result<(), BlockError>;

    fn flush(&mut self) -> Result<(), BlockError>;

    fn validate_read(&self, range: BlockRange, buffer: &[u8]) -> Result<(), BlockError> {
        self.state().ensure_available()?;
        let expected = self.info().expected_buffer_len(range)?;
        if buffer.len() == expected {
            Ok(())
        } else {
            Err(BlockError::BufferSizeMismatch)
        }
    }

    fn validate_write(&self, range: BlockRange, data: &[u8]) -> Result<(), BlockError> {
        self.state().ensure_available()?;
        let info = self.info();
        if info.read_only {
            return Err(BlockError::ReadOnly);
        }

        let expected = info.expected_buffer_len(range)?;
        if data.len() == expected {
            Ok(())
        } else {
            Err(BlockError::BufferSizeMismatch)
        }
    }
}

/// Completion record returned by queue and scheduler operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockCompletion {
    request_id: BlockRequestId,
    device_id: BlockDeviceId,
    result: Result<BlockResponse, BlockError>,
}

impl BlockCompletion {
    pub const fn succeeded(response: BlockResponse) -> Self {
        Self {
            request_id: response.request_id,
            device_id: response.device_id,
            result: Ok(response),
        }
    }

    pub const fn failed(
        request_id: BlockRequestId,
        device_id: BlockDeviceId,
        error: BlockError,
    ) -> Self {
        Self {
            request_id,
            device_id,
            result: Err(error),
        }
    }

    pub const fn request_id(&self) -> BlockRequestId {
        self.request_id
    }

    pub const fn device_id(&self) -> BlockDeviceId {
        self.device_id
    }

    pub const fn result(&self) -> &Result<BlockResponse, BlockError> {
        &self.result
    }
}

/// FIFO queue for request submission and completion collection.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BlockQueue {
    next_id: u64,
    pending: VecDeque<BlockRequest>,
    completed: VecDeque<BlockCompletion>,
}

impl BlockQueue {
    pub const fn new() -> Self {
        Self {
            next_id: 1,
            pending: VecDeque::new(),
            completed: VecDeque::new(),
        }
    }

    pub fn submit(&mut self, device_id: BlockDeviceId, kind: BlockRequestKind) -> BlockRequestId {
        let id = BlockRequestId::new(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.pending
            .push_back(BlockRequest::new(id, device_id, kind));
        id
    }

    pub fn pop_next(&mut self) -> Option<BlockRequest> {
        self.pending.pop_front()
    }

    pub fn complete(&mut self, completion: BlockCompletion) {
        self.completed.push_back(completion);
    }

    pub fn take_completion(&mut self) -> Option<BlockCompletion> {
        self.completed.pop_front()
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn completed_len(&self) -> usize {
        self.completed.len()
    }

    pub fn is_idle(&self) -> bool {
        self.pending.is_empty()
    }
}

/// Simple FIFO scheduler for dispatching queued requests to a selected block device.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BlockScheduler {
    queue: BlockQueue,
}

impl BlockScheduler {
    pub const fn new() -> Self {
        Self {
            queue: BlockQueue::new(),
        }
    }

    pub fn queue(&self) -> &BlockQueue {
        &self.queue
    }

    pub fn queue_mut(&mut self) -> &mut BlockQueue {
        &mut self.queue
    }

    pub fn submit(&mut self, device_id: BlockDeviceId, kind: BlockRequestKind) -> BlockRequestId {
        self.queue.submit(device_id, kind)
    }

    pub fn dispatch_next<D: BlockDevice>(
        &mut self,
        device: &mut D,
    ) -> Result<BlockRequestId, BlockError> {
        let Some(request) = self.queue.pop_next() else {
            return Err(BlockError::QueueEmpty);
        };

        let request_id = request.id();
        let device_id = request.device_id();
        let completion = if device.info().id != device_id {
            BlockCompletion::failed(request_id, device_id, BlockError::DeviceMismatch)
        } else {
            Self::execute(request, device)
        };

        self.queue.complete(completion);
        Ok(request_id)
    }

    pub fn take_completion(&mut self) -> Option<BlockCompletion> {
        self.queue.take_completion()
    }

    fn execute<D: BlockDevice>(request: BlockRequest, device: &mut D) -> BlockCompletion {
        let request_id = request.id();
        let device_id = request.device_id();

        let result = match request.into_kind() {
            BlockRequestKind::Read(range) => {
                let len = match device.info().expected_buffer_len(range) {
                    Ok(len) => len,
                    Err(error) => return BlockCompletion::failed(request_id, device_id, error),
                };
                let mut data = vec![0; len];
                device.read_blocks(range, &mut data).map(|()| {
                    BlockResponse::new(request_id, device_id, BlockResponseKind::Read { data })
                })
            }
            BlockRequestKind::Write { range, data } => {
                device.write_blocks(range, &data).map(|()| {
                    BlockResponse::new(
                        request_id,
                        device_id,
                        BlockResponseKind::Write { bytes: data.len() },
                    )
                })
            }
            BlockRequestKind::Flush => device
                .flush()
                .map(|()| BlockResponse::new(request_id, device_id, BlockResponseKind::Flush)),
        };

        match result {
            Ok(response) => BlockCompletion::succeeded(response),
            Err(error) => BlockCompletion::failed(request_id, device_id, error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockBlockDevice {
        info: BlockDeviceInfo,
        state: BlockDeviceState,
        storage: Vec<u8>,
        flush_count: usize,
    }

    impl MockBlockDevice {
        fn new(sectors: u64, block_size: u32) -> Self {
            let block_size = BlockSize::new(block_size).unwrap();
            let bytes = sectors as usize * block_size.bytes_usize();
            Self {
                info: BlockDeviceInfo::new(
                    BlockDeviceId::new(7),
                    block_size,
                    SectorCount::new(sectors),
                    false,
                    true,
                ),
                state: BlockDeviceState::Online,
                storage: vec![0; bytes],
                flush_count: 0,
            }
        }

        fn byte_bounds(&self, range: BlockRange) -> (usize, usize) {
            let start = range.start().get() as usize * self.info.block_size.bytes_usize();
            let len = range.byte_len(self.info.block_size).unwrap();
            (start, start + len)
        }
    }

    impl BlockDevice for MockBlockDevice {
        fn info(&self) -> BlockDeviceInfo {
            self.info
        }

        fn state(&self) -> BlockDeviceState {
            self.state
        }

        fn read_blocks(&mut self, range: BlockRange, buffer: &mut [u8]) -> Result<(), BlockError> {
            self.validate_read(range, buffer)?;
            let (start, end) = self.byte_bounds(range);
            buffer.copy_from_slice(&self.storage[start..end]);
            Ok(())
        }

        fn write_blocks(&mut self, range: BlockRange, data: &[u8]) -> Result<(), BlockError> {
            self.validate_write(range, data)?;
            let (start, end) = self.byte_bounds(range);
            self.storage[start..end].copy_from_slice(data);
            Ok(())
        }

        fn flush(&mut self) -> Result<(), BlockError> {
            self.state.ensure_available()?;
            self.flush_count += 1;
            Ok(())
        }
    }

    #[test]
    fn block_range_validation_rejects_empty_overflow_and_out_of_bounds() {
        let sectors = SectorCount::new(8);

        assert_eq!(
            BlockRange::new(Lba::new(0), SectorCount::new(0)).validate_within(sectors),
            Err(BlockError::EmptyRange)
        );
        assert_eq!(
            BlockRange::new(Lba::new(u64::MAX), SectorCount::new(1)).validate_within(sectors),
            Err(BlockError::RangeOverflow)
        );
        assert_eq!(
            BlockRange::new(Lba::new(7), SectorCount::new(2)).validate_within(sectors),
            Err(BlockError::OutOfBounds)
        );
        assert_eq!(
            BlockRange::new(Lba::new(6), SectorCount::new(2)).validate_within(sectors),
            Ok(())
        );
    }

    #[test]
    fn read_write_bounds_validate_buffer_sizes_and_device_limits() {
        let mut device = MockBlockDevice::new(4, 512);
        let range = BlockRange::new(Lba::new(1), SectorCount::new(2));
        let in_bounds = vec![1; 1024];
        let mut out = vec![0; 1024];

        assert_eq!(device.write_blocks(range, &in_bounds), Ok(()));
        assert_eq!(device.read_blocks(range, &mut out), Ok(()));
        assert_eq!(out, in_bounds);

        let mut short = vec![0; 512];
        assert_eq!(
            device.read_blocks(range, &mut short),
            Err(BlockError::BufferSizeMismatch)
        );
        assert_eq!(
            device.write_blocks(
                BlockRange::new(Lba::new(3), SectorCount::new(2)),
                &in_bounds
            ),
            Err(BlockError::OutOfBounds)
        );
    }

    #[test]
    fn queue_submit_and_complete_preserves_fifo_order() {
        let mut queue = BlockQueue::new();
        let device_id = BlockDeviceId::new(1);
        let first = queue.submit(
            device_id,
            BlockRequestKind::Read(BlockRange::new(Lba::new(0), SectorCount::new(1))),
        );
        let second = queue.submit(device_id, BlockRequestKind::Flush);

        assert_eq!(first, BlockRequestId::new(1));
        assert_eq!(second, BlockRequestId::new(2));
        assert_eq!(queue.pending_len(), 2);
        assert_eq!(queue.pop_next().unwrap().id(), first);

        let response = BlockResponse::new(first, device_id, BlockResponseKind::Flush);
        queue.complete(BlockCompletion::succeeded(response));
        assert_eq!(queue.completed_len(), 1);
        assert_eq!(queue.take_completion().unwrap().request_id(), first);
        assert_eq!(queue.pop_next().unwrap().id(), second);
    }

    #[test]
    fn mock_block_device_read_write_round_trip_through_scheduler() {
        let mut device = MockBlockDevice::new(4, 4);
        let mut scheduler = BlockScheduler::new();
        let range = BlockRange::new(Lba::new(1), SectorCount::new(2));
        let data = vec![9, 8, 7, 6, 5, 4, 3, 2];

        let write_id = scheduler.submit(
            device.info().id,
            BlockRequestKind::Write {
                range,
                data: data.clone(),
            },
        );
        assert_eq!(scheduler.dispatch_next(&mut device), Ok(write_id));
        assert_eq!(
            scheduler.take_completion().unwrap().result(),
            &Ok(BlockResponse::new(
                write_id,
                device.info().id,
                BlockResponseKind::Write { bytes: data.len() }
            ))
        );

        let read_id = scheduler.submit(device.info().id, BlockRequestKind::Read(range));
        assert_eq!(scheduler.dispatch_next(&mut device), Ok(read_id));
        assert_eq!(
            scheduler.take_completion().unwrap().result(),
            &Ok(BlockResponse::new(
                read_id,
                device.info().id,
                BlockResponseKind::Read { data }
            ))
        );
    }

    #[test]
    fn flush_behavior_completes_and_updates_device_state() {
        let mut device = MockBlockDevice::new(2, 512);
        let mut scheduler = BlockScheduler::new();

        let flush_id = scheduler.submit(device.info().id, BlockRequestKind::Flush);
        assert_eq!(scheduler.dispatch_next(&mut device), Ok(flush_id));

        let completion = scheduler.take_completion().unwrap();
        assert_eq!(
            completion.result(),
            &Ok(BlockResponse::new(
                flush_id,
                device.info().id,
                BlockResponseKind::Flush
            ))
        );
        assert_eq!(device.flush_count, 1);
    }
}
