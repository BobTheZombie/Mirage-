//! Fixed-size network service IPC ABI.
//!
//! These request messages are intentionally plain `repr(C)` values with no
//! borrowed data, allocation, or architecture-dependent padding requirements.
//! L1 validates descriptor ownership and user pointer ranges, then forwards one
//! of these messages to the registered network service endpoint.

use crate::kernel::process::ProcessId;

pub const NETWORK_SERVICE_ABI_VERSION: u16 = 1;

#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkOpcode {
    Socket = 1,
    Bind = 2,
    Listen = 3,
    Accept = 4,
    Connect = 5,
    Sendmsg = 6,
    Recvmsg = 7,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkRequestHeader {
    pub abi_version: u16,
    pub opcode: u16,
    pub flags: u32,
    pub client_pid: u64,
    pub request_id: u64,
}

impl NetworkRequestHeader {
    pub const fn new(
        opcode: NetworkOpcode,
        client: ProcessId,
        request_id: u64,
        flags: u32,
    ) -> Self {
        Self {
            abi_version: NETWORK_SERVICE_ABI_VERSION,
            opcode: opcode as u16,
            flags,
            client_pid: client.raw(),
            request_id,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkSocketRequest {
    pub header: NetworkRequestHeader,
    pub socket_handle: u64,
    pub domain: i32,
    pub socket_type: i32,
    pub protocol: i32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkSockaddrRequest {
    pub header: NetworkRequestHeader,
    pub socket_handle: u64,
    pub addr_ptr: u64,
    pub addr_len: u32,
    pub value: i32,
    pub result_addr_len_ptr: u64,
    pub accepted_socket_handle: u64,
}

pub type NetworkBindRequest = NetworkSockaddrRequest;
pub type NetworkListenRequest = NetworkSockaddrRequest;
pub type NetworkAcceptRequest = NetworkSockaddrRequest;
pub type NetworkConnectRequest = NetworkSockaddrRequest;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkSendmsgRequest {
    pub header: NetworkRequestHeader,
    pub socket_handle: u64,
    pub message_ptr: u64,
    pub flags: u64,
    pub reserved: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkRecvmsgRequest {
    pub header: NetworkRequestHeader,
    pub socket_handle: u64,
    pub message_ptr: u64,
    pub flags: u64,
    pub reserved: u64,
}

pub trait NetworkIpcRequest {
    fn as_bytes(&self) -> &[u8];
}

macro_rules! impl_network_ipc_request {
    ($type:ty) => {
        impl NetworkIpcRequest for $type {
            fn as_bytes(&self) -> &[u8] {
                unsafe {
                    core::slice::from_raw_parts(
                        (self as *const Self).cast::<u8>(),
                        core::mem::size_of::<Self>(),
                    )
                }
            }
        }
    };
}

impl_network_ipc_request!(NetworkSocketRequest);
impl_network_ipc_request!(NetworkSockaddrRequest);
impl_network_ipc_request!(NetworkSendmsgRequest);
impl_network_ipc_request!(NetworkRecvmsgRequest);

const _: () = assert!(core::mem::size_of::<NetworkSocketRequest>() <= 64);
const _: () = assert!(core::mem::size_of::<NetworkSockaddrRequest>() <= 64);
const _: () = assert!(core::mem::size_of::<NetworkSendmsgRequest>() <= 64);
const _: () = assert!(core::mem::size_of::<NetworkRecvmsgRequest>() <= 64);
