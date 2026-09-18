//! Lossless protocol-28 Huffman image jobs, checked against the native SDK.
use std::{cmp::Reverse, collections::BinaryHeap};

use anyhow::{Result, ensure};
use deku::{
    bitvec::{BitVec, Msb0},
    prelude::*,
};

use crate::{protocol::Frame, raster::Raster};

type Bits = BitVec<u8, Msb0>;

struct Node {
    name: u16,
    children: Option<(usize, usize)>,
}
struct Tree {
    nodes: Vec<Node>,
    codes: [Bits; 256],
}
impl Tree {
    fn new(bytes: &[u8]) -> Self {
        let mut frequencies = [0usize; 256];
        for byte in bytes {
            frequencies[*byte as usize] += 1;
        }
        let mut nodes = Vec::new();
        let mut heap = BinaryHeap::new();
        for (name, weight) in frequencies.into_iter().enumerate() {
            if weight != 0 {
                heap.push(Reverse((weight, nodes.len())));
                nodes.push(Node {
                    name: name as u16,
                    children: None,
                });
            }
        }
        if nodes.len() == 1 {
            let symbol = nodes[0].name;
            let neighbor = if symbol < 128 { symbol + 1 } else { symbol - 1 };
            heap.push(Reverse((1, nodes.len())));
            nodes.push(Node {
                name: neighbor,
                children: None,
            });
        }
        let leaves = nodes.len();
        while heap.len() > 1 {
            // SDK scans nodes in insertion order; ties choose the earliest.
            let Reverse((lw, left)) = heap.pop().unwrap();
            let Reverse((rw, right)) = heap.pop().unwrap();
            heap.push(Reverse((lw + rw, nodes.len())));
            nodes.push(Node {
                name: (256 + nodes.len() - leaves) as u16,
                children: Some((left, right)),
            });
        }
        let mut tree = Self {
            nodes,
            codes: std::array::from_fn(|_| Bits::new()),
        };
        tree.assign_codes(tree.nodes.len() - 1, &mut Bits::new());
        tree
    }
    fn assign_codes(&mut self, index: usize, path: &mut Bits) {
        if let Some((left, right)) = self.nodes[index].children {
            path.push(false);
            self.assign_codes(left, path);
            path.pop();
            path.push(true);
            self.assign_codes(right, path);
            path.pop();
        } else {
            self.codes[self.nodes[index].name as usize] = path.clone();
        }
    }
    fn traversal(&self, index: usize, preorder: bool, bytes: &mut Vec<u8>) {
        let node = &self.nodes[index];
        if preorder {
            bytes.extend(node.name.to_le_bytes());
        }
        if let Some((left, _)) = node.children {
            self.traversal(left, preorder, bytes);
        }
        if !preorder {
            bytes.extend(node.name.to_le_bytes());
        }
        if let Some((_, right)) = node.children {
            self.traversal(right, preorder, bytes);
        }
    }
    fn payload(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.nodes.len() * 4);
        self.traversal(self.nodes.len() - 1, true, &mut bytes);
        self.traversal(self.nodes.len() - 1, false, &mut bytes);
        bytes
    }
}

#[derive(DekuWrite)]
#[deku(endian = "little")]
struct DataHeader {
    index: u16,
    /// Bytes following index and this length field.
    length: u16,
    format: u8,
    byte_width: u16,
    reserved: u32,
}

fn frame(subclass: u8, payload: Vec<u8>) -> Frame {
    Frame {
        mode: 0,
        message_type: 1,
        sub_message_type: u8::from(subclass != 0x0d),
        superclass: 5,
        subclass,
        payload,
    }
}

/// Encode start, tree/data sections, and end. `buffer_length` is the printer's
/// reported pBuffLength, NOT the receive codec's maximum inner-body length.
/// This produces image data only: copies, density, feed, pacing and print status
/// belong to the surrounding print session and are not included here.
pub fn encode_image(raster: &Raster, buffer_length: u16) -> Result<Vec<Frame>> {
    ensure!(
        buffer_length >= 64,
        "Image buffer length must be at least 64 bytes"
    );
    let byte_width = raster.width() as usize / 8;
    let mut remaining = raster.bytes();
    let mut job = vec![frame(0x0b, vec![])];
    while !remaining.is_empty() {
        let mut count = remaining.len();
        let tree = loop {
            let tree = Tree::new(&remaining[..count]);
            // Native getAllCompressImagePart uses a strict inequality here.
            if tree.nodes.len() * 4 < usize::from(buffer_length) - 25 {
                break tree;
            }
            count = (count / 2 / byte_width) * byte_width;
            ensure!(
                count > 0,
                "Compression tree for one row cannot fit buffer length {buffer_length}"
            );
        };
        job.push(frame(0x0e, tree.payload()));
        encode_section(
            &remaining[..count],
            byte_width,
            buffer_length,
            &tree,
            &mut job,
        )?;
        remaining = &remaining[count..];
    }
    job.push(frame(0x0c, vec![]));
    // The SDK's unusual packet budget fallback can exceed the negotiated limit.
    // Reject such configurations instead of returning a job the printer cannot buffer.
    for frame in &job {
        ensure!(
            frame.payload.len() + 15 <= usize::from(buffer_length),
            "SDK image packet would exceed buffer length {buffer_length}; use a larger buffer or narrower raster"
        );
    }
    Ok(job)
}

fn encode_section(
    bytes: &[u8],
    byte_width: usize,
    buffer_length: u16,
    tree: &Tree,
    job: &mut Vec<Frame>,
) -> Result<()> {
    // Preserve the SDK's units, even though the function name suggests bytes:
    // floor((pBuffLength - 36)/byte_width), or byte_width^2 * 10 if <= 11.
    let hint = (usize::from(buffer_length) - 36) / byte_width;
    let hint = if hint > 11 {
        hint
    } else {
        byte_width * byte_width * 10
    };
    let bit_limit = hint.min(bytes.len()) * 8 - 8;
    let mut bits = Bits::new();
    let mut packet_bits = Vec::new();
    let mut current = 0;
    for row in bytes.chunks_exact(byte_width) {
        let row_bits: usize = row
            .iter()
            .map(|byte| tree.codes[*byte as usize].len())
            .sum();
        if current + row_bits >= bit_limit {
            ensure!(
                current > 0,
                "SDK would emit an empty packet for this raster/buffer combination; increase image size or buffer length"
            );
            packet_bits.push(current);
            current = 0;
        }
        current += row_bits;
        for byte in row {
            bits.extend_from_bitslice(&tree.codes[*byte as usize]);
        }
    }
    packet_bits.push(current);
    ensure!(
        packet_bits.len() <= usize::from(u16::MAX) + 1,
        "Too many image packets in one tree section"
    );
    let mut start = 0;
    for (index, bit_count) in packet_bits.into_iter().enumerate() {
        let encoded_bytes = bit_count.div_ceil(8);
        let padding = encoded_bytes * 8 - bit_count;
        let compressed_len = encoded_bytes + 1;
        ensure!(
            compressed_len + 11 <= crate::protocol::MAX_PAYLOAD_LEN,
            "Compressed image payload exceeds the wire limit"
        );
        let mut payload = DataHeader {
            index: index as u16,
            length: (compressed_len + 7) as u16,
            format: 0x11,
            byte_width: byte_width as u16,
            reserved: 0,
        }
        .to_bytes()?;
        payload.push(padding as u8);
        // Native compression packs from a contiguous bit array: ignored padding
        // at a packet boundary contains the NEXT packet's bits, not necessarily 0.
        let mut packed = bits[start..(start + encoded_bytes * 8).min(bits.len())].to_bitvec();
        packed.resize(encoded_bytes * 8, false);
        packed.force_align();
        payload.extend(packed.into_vec());
        job.push(frame(0x0d, payload));
        start += bit_count;
    }
    Ok(())
}
