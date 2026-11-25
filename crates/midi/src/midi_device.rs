//! MIDI input device enumeration and hardware connection.
//!
//! On macOS, uses CoreMIDI directly via FFI (zero external crate dependencies)
//! to enumerate external MIDI keyboards and stream incoming MIDI 1.0 messages
//! lock-free into a [`MidiInputRing`].  Also provides [`VirtualMidiInput`] so
//! desktop UI keyboards and tests can inject MIDI 1.0 events identically.

use crate::midi_event::{MidiError, MidiMessage};
use crate::midi_recording::MidiInputRing;
use std::sync::Arc;

/// Descriptor for a discovered MIDI input port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiPortInfo {
    pub index: usize,
    pub name: String,
}

/// Virtual / software MIDI input emitter sharing the same [`MidiInputRing`].
#[derive(Clone)]
pub struct VirtualMidiInput {
    ring: Arc<MidiInputRing>,
}

impl VirtualMidiInput {
    pub fn new(ring: Arc<MidiInputRing>) -> Self {
        Self { ring }
    }

    pub fn send(&self, msg: MidiMessage) -> bool {
        self.ring.push(msg)
    }
}

/// Active MIDI input connection handle. Dropping it disconnects the port.
pub struct MidiInputConnection {
    #[cfg(target_os = "macos")]
    _inner: macos_coremidi::CoreMidiConnection,
    #[cfg(not(target_os = "macos"))]
    _port_name: String,
}

/// Enumerates and opens hardware MIDI input ports.
pub struct MidiInputManager;

impl MidiInputManager {
    /// Discover currently available external MIDI input ports.
    pub fn enumerate_input_ports() -> Result<Vec<MidiPortInfo>, MidiError> {
        #[cfg(target_os = "macos")]
        {
            Ok(macos_coremidi::enumerate_sources())
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok(Vec::new())
        }
    }

    /// Connect to a hardware MIDI input port by index and stream incoming MIDI 1.0
    /// messages into `ring`.
    pub fn open_input_port(
        port_index: usize,
        ring: Arc<MidiInputRing>,
    ) -> Result<MidiInputConnection, MidiError> {
        #[cfg(target_os = "macos")]
        {
            let conn = macos_coremidi::open_source(port_index, ring)?;
            Ok(MidiInputConnection { _inner: conn })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (port_index, ring);
            Err(MidiError::DeviceUnavailable(
                "Hardware MIDI input is not supported on this platform".to_string(),
            ))
        }
    }
}

#[cfg(target_os = "macos")]
mod macos_coremidi {
    use super::*;
    use std::ffi::{CStr, c_char, c_void};
    use std::ptr;

    type OSStatus = i32;
    type MIDIClientRef = u32;
    type MIDIPortRef = u32;
    type MIDIEndpointRef = u32;
    type CFStringRef = *const c_void;
    type CFAllocatorRef = *const c_void;

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    #[repr(C)]
    struct MIDIPacketListHeader {
        num_packets: u32,
    }

    type MIDIReadProc = extern "C" fn(
        pkt_list: *const MIDIPacketListHeader,
        read_proc_ref_con: *mut c_void,
        src_conn_ref_con: *mut c_void,
    );

    #[link(name = "CoreMIDI", kind = "framework")]
    unsafe extern "C" {
        fn MIDIGetNumberOfSources() -> usize;
        fn MIDIGetSource(source_index_0: usize) -> MIDIEndpointRef;
        fn MIDIObjectGetStringProperty(
            obj: u32,
            property_id: CFStringRef,
            str_out: *mut CFStringRef,
        ) -> OSStatus;
        fn MIDIClientCreate(
            name: CFStringRef,
            notify_proc: *const c_void,
            notify_ref_con: *mut c_void,
            out_client: *mut MIDIClientRef,
        ) -> OSStatus;
        fn MIDIClientDispose(client: MIDIClientRef) -> OSStatus;
        fn MIDIInputPortCreate(
            client: MIDIClientRef,
            port_name: CFStringRef,
            read_proc: MIDIReadProc,
            ref_con: *mut c_void,
            out_port: *mut MIDIPortRef,
        ) -> OSStatus;
        fn MIDIPortDispose(port: MIDIPortRef) -> OSStatus;
        fn MIDIPortConnectSource(
            port: MIDIPortRef,
            source: MIDIEndpointRef,
            conn_ref_con: *mut c_void,
        ) -> OSStatus;

        static kMIDIPropertyDisplayName: CFStringRef;
        static kMIDIPropertyName: CFStringRef;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            alloc: CFAllocatorRef,
            c_str: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFStringGetCString(
            the_string: CFStringRef,
            buffer: *mut c_char,
            buffer_size: isize,
            encoding: u32,
        ) -> u8;
        fn CFRelease(cf: *const c_void);
    }

    fn endpoint_name(ep: MIDIEndpointRef, fallback_idx: usize) -> String {
        unsafe {
            let mut cf_name: CFStringRef = ptr::null();
            let mut status =
                MIDIObjectGetStringProperty(ep, kMIDIPropertyDisplayName, &mut cf_name);
            if status != 0 || cf_name.is_null() {
                status = MIDIObjectGetStringProperty(ep, kMIDIPropertyName, &mut cf_name);
            }
            if status == 0 && !cf_name.is_null() {
                let mut buf = [0i8; 256];
                let ok = CFStringGetCString(
                    cf_name,
                    buf.as_mut_ptr(),
                    buf.len() as isize,
                    K_CF_STRING_ENCODING_UTF8,
                );
                CFRelease(cf_name);
                if ok != 0 {
                    let s = CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
                    if !s.is_empty() {
                        return s;
                    }
                }
            }
        }
        format!("MIDI Input {fallback_idx}")
    }

    pub fn enumerate_sources() -> Vec<MidiPortInfo> {
        let count = unsafe { MIDIGetNumberOfSources() };
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let ep = unsafe { MIDIGetSource(i) };
            if ep != 0 {
                out.push(MidiPortInfo {
                    index: i,
                    name: endpoint_name(ep, i),
                });
            }
        }
        out
    }

    extern "C" fn midi_read_callback(
        pkt_list: *const MIDIPacketListHeader,
        read_proc_ref_con: *mut c_void,
        _src_conn_ref_con: *mut c_void,
    ) {
        if pkt_list.is_null() || read_proc_ref_con.is_null() {
            return;
        }
        // SAFETY: `read_proc_ref_con` points to an `Arc<MidiInputRing>` kept alive by CoreMidiConnection.
        let ring = unsafe { &*(read_proc_ref_con as *const MidiInputRing) };
        let num_packets = unsafe { (*pkt_list).num_packets };
        let mut ptr_byte = unsafe { (pkt_list as *const u8).add(4) };

        for _ in 0..num_packets {
            // SAFETY: CoreMIDI MIDIPacket has 8-byte timestamp + 2-byte length, followed by `length` data bytes.
            let length = unsafe {
                let len_ptr = ptr_byte.add(8) as *const u16;
                ptr::read_unaligned(len_ptr) as usize
            };
            let data_ptr = unsafe { ptr_byte.add(10) };
            let data = unsafe { std::slice::from_raw_parts(data_ptr, length) };

            // Parse MIDI 1.0 channel messages inside this packet.
            let mut i = 0;
            while i < data.len() {
                let status = data[i];
                if (0x80..0xF0).contains(&status) {
                    let msg_len = if (status & 0xF0) == 0xC0 || (status & 0xF0) == 0xD0 {
                        2
                    } else {
                        3
                    };
                    if i + msg_len <= data.len() {
                        if let Ok(msg) = MidiMessage::from_bytes(&data[i..i + msg_len]) {
                            let _ = ring.push(msg);
                        }
                        i += msg_len;
                    } else {
                        break;
                    }
                } else {
                    i += 1;
                }
            }

            // Advance to next MIDIPacket (4-byte aligned on ARM64 / Apple Silicon).
            let packet_bytes = (10 + length + 3) & !3;
            ptr_byte = unsafe { ptr_byte.add(packet_bytes) };
        }
    }

    pub struct CoreMidiConnection {
        client: MIDIClientRef,
        port: MIDIPortRef,
        ring_ptr: *const MidiInputRing,
    }

    // SAFETY: `ring_ptr` comes from `Arc::into_raw` and `MidiInputRing` is Sync + Send.
    unsafe impl Send for CoreMidiConnection {}

    impl Drop for CoreMidiConnection {
        fn drop(&mut self) {
            unsafe {
                if self.port != 0 {
                    MIDIPortDispose(self.port);
                }
                if self.client != 0 {
                    MIDIClientDispose(self.client);
                }
                if !self.ring_ptr.is_null() {
                    drop(Arc::from_raw(self.ring_ptr));
                }
            }
        }
    }

    pub fn open_source(
        port_index: usize,
        ring: Arc<MidiInputRing>,
    ) -> Result<CoreMidiConnection, MidiError> {
        let count = unsafe { MIDIGetNumberOfSources() };
        if port_index >= count {
            return Err(MidiError::PortNotFound(format!("port index {port_index}")));
        }
        let ep = unsafe { MIDIGetSource(port_index) };
        if ep == 0 {
            return Err(MidiError::PortNotFound(format!("port index {port_index}")));
        }

        let ring_ptr = Arc::into_raw(ring);
        let mut client: MIDIClientRef = 0;
        let mut port: MIDIPortRef = 0;

        unsafe {
            let c_name = c"EsperiaMIDI";
            let cf_client_name =
                CFStringCreateWithCString(ptr::null(), c_name.as_ptr(), K_CF_STRING_ENCODING_UTF8);
            let status =
                MIDIClientCreate(cf_client_name, ptr::null(), ptr::null_mut(), &mut client);
            CFRelease(cf_client_name);
            if status != 0 {
                drop(Arc::from_raw(ring_ptr));
                return Err(MidiError::DeviceConnectionFailed(format!(
                    "MIDIClientCreate failed ({status})"
                )));
            }

            let p_name = c"EsperiaInput";
            let cf_port_name =
                CFStringCreateWithCString(ptr::null(), p_name.as_ptr(), K_CF_STRING_ENCODING_UTF8);
            let status = MIDIInputPortCreate(
                client,
                cf_port_name,
                midi_read_callback,
                ring_ptr as *mut c_void,
                &mut port,
            );
            CFRelease(cf_port_name);
            if status != 0 {
                MIDIClientDispose(client);
                drop(Arc::from_raw(ring_ptr));
                return Err(MidiError::DeviceConnectionFailed(format!(
                    "MIDIInputPortCreate failed ({status})"
                )));
            }

            let status = MIDIPortConnectSource(port, ep, ptr::null_mut());
            if status != 0 {
                MIDIPortDispose(port);
                MIDIClientDispose(client);
                drop(Arc::from_raw(ring_ptr));
                return Err(MidiError::DeviceConnectionFailed(format!(
                    "MIDIPortConnectSource failed ({status})"
                )));
            }
        }

        Ok(CoreMidiConnection {
            client,
            port,
            ring_ptr,
        })
    }
}
