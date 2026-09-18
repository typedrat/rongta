use bytes::BytesMut;
use rongta::{
    image_data::encode_image,
    protocol::{Frame, FrameCodec},
    raster::Raster,
};
use serde_json::Value;
use tokio_util::codec::Decoder;

fn corpus() -> Value {
    serde_json::from_str(include_str!("fixtures/sdk/image.json")).unwrap()
}
fn frames(wire: &[u8]) -> Vec<Frame> {
    let mut bytes = BytesMut::from(wire);
    let mut codec = FrameCodec::new(u16::MAX);
    let mut out = vec![];
    while let Some(frame) = codec.decode_eof(&mut bytes).unwrap() {
        out.push(frame);
    }
    out
}
fn raster(v: &Value) -> Raster {
    Raster::new(
        v["width"].as_u64().unwrap() as u32,
        v["height"].as_u64().unwrap() as u32,
        hex::decode(v["raster_hex"].as_str().unwrap()).unwrap(),
    )
    .unwrap()
}

#[test]
fn jobs_match_native_sdk_bytes_including_tree_ties_and_packet_boundaries() {
    let corpus = corpus();
    for v in corpus["image_vectors"]
        .as_array()
        .unwrap()
        .iter()
        .chain(corpus["raster_vectors"].as_array().unwrap())
    {
        if v.get("unsupported_reason").is_some() {
            continue;
        }
        let job = encode_image(&raster(v), v["buffer_length"].as_u64().unwrap() as u16).unwrap();
        let expected = frames(&hex::decode(v["wire_hex"].as_str().unwrap()).unwrap());
        assert_eq!(job, expected, "{}", v["name"]);
        for frame in &job {
            assert!(frame.encode().unwrap().len() <= v["buffer_length"].as_u64().unwrap() as usize);
        }
    }
}

// Independent decoder: reconstruct the transmitted tree from its traversals,
// without sharing the encoder's tree builder or bit packing implementation.
#[derive(Debug)]
enum Tree {
    Leaf(u8),
    Branch(Box<Tree>, Box<Tree>),
}
fn tree(preorder: &[u16], inorder: &[u16]) -> Tree {
    assert_eq!(preorder.len(), inorder.len());
    let root = preorder[0];
    if root < 256 {
        assert_eq!(preorder.len(), 1);
        return Tree::Leaf(root as u8);
    }
    let split = inorder.iter().position(|name| *name == root).unwrap();
    Tree::Branch(
        Box::new(tree(&preorder[1..split + 1], &inorder[..split])),
        Box::new(tree(&preorder[split + 1..], &inorder[split + 1..])),
    )
}
fn decode(job: &[Frame]) -> Vec<u8> {
    assert_eq!(job.first().unwrap().subclass, 0x0b);
    assert_eq!(job.last().unwrap().subclass, 0x0c);
    let mut root = None;
    let mut output = vec![];
    let mut index = 0u16;
    for frame in &job[1..job.len() - 1] {
        let p = &frame.payload;
        match frame.subclass {
            0x0e => {
                assert_eq!(p.len() % 4, 0);
                let names: Vec<_> = p
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|n| u16::from_le_bytes([n[0], n[1]]))
                    .collect();
                root = Some(tree(&names[..names.len() / 2], &names[names.len() / 2..]));
                index = 0;
            }
            0x0d => {
                assert_eq!(u16::from_le_bytes([p[0], p[1]]), index);
                index += 1;
                assert_eq!(u16::from_le_bytes([p[2], p[3]]) as usize, p.len() - 4);
                assert_eq!(p[4], 0x11);
                assert_eq!(&p[7..11], &[0; 4]);
                let width = u16::from_le_bytes([p[5], p[6]]) as usize;
                let padding = p[11] as usize;
                assert!(padding < 8);
                let bit_length = (p.len() - 12) * 8 - padding;
                let root = root.as_ref().unwrap();
                let mut node = root;
                let start = output.len();
                for i in 0..bit_length {
                    node = match node {
                        Tree::Branch(left, right) => {
                            if p[12 + i / 8] & (128 >> (i % 8)) == 0 {
                                left
                            } else {
                                right
                            }
                        }
                        Tree::Leaf(_) => panic!("encoder left a leaf unconsumed"),
                    };
                    if let Tree::Leaf(byte) = node {
                        output.push(*byte);
                        node = root;
                    }
                }
                assert!(std::ptr::eq(node, root), "partial Huffman code");
                assert_eq!((output.len() - start) % width, 0, "split raster row");
            }
            code => panic!("unexpected image command {code}"),
        }
    }
    output
}

#[test]
fn independently_decoding_sdk_and_rust_jobs_recovers_original_rasters() {
    for v in corpus()["image_vectors"].as_array().unwrap() {
        if v.get("unsupported_reason").is_some() {
            continue;
        }
        let raster = raster(v);
        let sdk = frames(&hex::decode(v["wire_hex"].as_str().unwrap()).unwrap());
        assert_eq!(decode(&sdk), raster.bytes(), "SDK {}", v["name"]);
        let encoded = encode_image(&raster, v["buffer_length"].as_u64().unwrap() as u16).unwrap();
        assert_eq!(decode(&encoded), raster.bytes(), "Rust {}", v["name"]);
    }
}

#[test]
fn invalid_budgets_and_native_empty_packet_edge_case_fail() {
    let raster = Raster::new(8, 1, vec![0x80]).unwrap();
    assert!(
        encode_image(&raster, 4096)
            .unwrap_err()
            .to_string()
            .contains("empty packet")
    );
    for budget in [0, 25, 36, 63] {
        assert!(encode_image(&raster, budget).is_err());
    }
    // A single row's tree cannot fit; do not reproduce the SDK's zero-length loop.
    let wide = Raster::new(2048, 1, (0..=255).collect()).unwrap();
    assert!(
        encode_image(&wide, 256)
            .unwrap_err()
            .to_string()
            .contains("one row")
    );
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(48))]
    #[test]
    fn arbitrary_rasters_survive_independent_huffman_decoding(
        (width, height, data) in (1usize..=32, 16usize..=64).prop_flat_map(|(w,h)|
            (proptest::strategy::Just(w), proptest::strategy::Just(h), proptest::collection::vec(proptest::num::u8::ANY, w*h)))
    ) {
        let raster = Raster::new(width as u32 * 8, height as u32, data).unwrap();
        let job = encode_image(&raster, 4096).unwrap();
        proptest::prop_assert_eq!(decode(&job), raster.bytes());
    }
}
use proptest::strategy::Strategy;
