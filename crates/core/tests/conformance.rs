//! 协议一致性测试向量。
//!
//! 运行 `AB_GENERATE_VECTORS=1 cargo test -p audiobridge-core` 重新生成
//! `protocol/conformance-vectors.json`；默认模式下断言当前实现与已提交向量
//! 完全一致（字节级）。Kotlin 侧使用同一文件做双向验证——任何破坏线协议
//! 兼容性的改动都会在这里失败。

use std::path::PathBuf;

use audiobridge_core::codec::Codec;
use audiobridge_core::error::AckStatus;
use audiobridge_core::framing::{
    bye_payload, ping_payload, read_hello, read_hello_ack, send_hello, send_hello_ack,
    write_message, DataPayload, MSG_BYE, MSG_DATA, MSG_PING,
};
use audiobridge_core::hello::{Hello, HelloAck};
use serde::Serialize;

#[derive(Serialize, serde::Deserialize)]
struct Case {
    name: String,
    kind: String,
    bytes: String,
    fields: serde_json::Value,
}

#[derive(Serialize, serde::Deserialize)]
struct Vectors {
    protocol_version: u8,
    note: String,
    cases: Vec<Case>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hello_basic() -> Hello {
    Hello {
        device_name: "Pixel 8".into(),
        codec: Codec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        frame_ms: 20,
        buffer_target_ms: 80,
        auth_token_sha256: [0u8; 32],
    }
}

fn hello_ack(status: AckStatus) -> HelloAck {
    HelloAck {
        status,
        codec: Codec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        device_name: "DESKTOP-PC".into(),
        jitter_target_ms: 80,
    }
}

fn build_cases() -> Vec<Case> {
    let mut token = [0u8; 32];
    token[..4].copy_from_slice(&[0x01, 0x23, 0x45, 0x67]);

    let h1 = hello_basic();
    let h2 = Hello {
        auth_token_sha256: token,
        ..hello_basic()
    };
    let a_ok = hello_ack(AckStatus::Ok);
    let a_busy = hello_ack(AckStatus::Busy);

    let data = DataPayload {
        seq: 7,
        timestamp_ms: 12345,
        audio: (0u8..=31).collect(),
    };
    let data_frame = {
        let mut wire = Vec::new();
        write_message(&mut wire, MSG_DATA, &data.to_bytes()).unwrap();
        wire
    };

    let mut cases = vec![
        Case {
            name: "hello_basic".into(),
            kind: "hello".into(),
            bytes: hex(&h1.to_bytes()),
            fields: serde_json::json!({
                "device_name": h1.device_name,
                "codec": "pcm_s16le",
                "sample_rate": h1.sample_rate,
                "channels": h1.channels,
                "frame_ms": h1.frame_ms,
                "buffer_target_ms": h1.buffer_target_ms,
                "auth_token": null,
            }),
        },
        Case {
            name: "hello_with_token".into(),
            kind: "hello".into(),
            bytes: hex(&h2.to_bytes()),
            fields: serde_json::json!({
                "device_name": h2.device_name,
                "codec": "pcm_s16le",
                "sample_rate": h2.sample_rate,
                "channels": h2.channels,
                "frame_ms": h2.frame_ms,
                "buffer_target_ms": h2.buffer_target_ms,
                "auth_token": hex(&token),
            }),
        },
        Case {
            name: "hello_ack_ok".into(),
            kind: "hello_ack".into(),
            bytes: hex(&a_ok.to_bytes()),
            fields: serde_json::json!({
                "status": "ok",
                "codec": "pcm_s16le",
                "sample_rate": a_ok.sample_rate,
                "channels": a_ok.channels,
                "device_name": a_ok.device_name,
                "jitter_target_ms": a_ok.jitter_target_ms,
            }),
        },
        Case {
            name: "hello_ack_busy".into(),
            kind: "hello_ack".into(),
            bytes: hex(&a_busy.to_bytes()),
            fields: serde_json::json!({ "status": "busy" }),
        },
        Case {
            name: "data_payload".into(),
            kind: "data_payload".into(),
            bytes: hex(&data.to_bytes()),
            fields: serde_json::json!({
                "seq": data.seq,
                "timestamp_ms": data.timestamp_ms,
                "audio_len": data.audio.len(),
            }),
        },
        Case {
            name: "message_data".into(),
            kind: "message".into(),
            bytes: hex(&data_frame),
            fields: serde_json::json!({
                "msg_type": 3,
                "payload_len": data_frame.len() - 3,
            }),
        },
    ];

    let ping_frame = {
        let mut wire = Vec::new();
        write_message(&mut wire, MSG_PING, &ping_payload(0xDEAD_BEEF)).unwrap();
        wire
    };
    cases.push(Case {
        name: "message_ping".into(),
        kind: "message".into(),
        bytes: hex(&ping_frame),
        fields: serde_json::json!({ "msg_type": 4, "payload_len": 4 }),
    });

    let bye_frame = {
        let mut wire = Vec::new();
        write_message(&mut wire, MSG_BYE, &bye_payload(0, "bye")).unwrap();
        wire
    };
    cases.push(Case {
        name: "message_bye".into(),
        kind: "message".into(),
        bytes: hex(&bye_frame),
        fields: serde_json::json!({ "msg_type": 6, "payload_len": 4 }),
    });

    cases
}

fn vectors() -> Vectors {
    Vectors {
        protocol_version: 1,
        note: "由 audiobridge-core 的 conformance 测试生成；Rust 与 Kotlin 双侧共用。重新生成：AB_GENERATE_VECTORS=1 cargo test -p audiobridge-core".into(),
        cases: build_cases(),
    }
}

fn vectors_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../protocol/conformance-vectors.json")
}

#[test]
fn conformance_vectors_match() {
    let vectors = vectors();
    let path = vectors_path();

    if std::env::var("AB_GENERATE_VECTORS").as_deref() == Ok("1") {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&vectors).unwrap() + "\n",
        )
        .unwrap();
        eprintln!("generated {}", path.display());
        return;
    }

    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing vectors file {}", path.display()));
    let saved: Vectors = serde_json::from_str(&text).unwrap();
    assert_eq!(saved.protocol_version, 1);
    assert_eq!(saved.cases.len(), vectors.cases.len(), "case count changed; regenerate vectors");
    for (want, got) in vectors.cases.iter().zip(saved.cases.iter()) {
        assert_eq!(want.name, got.name, "case order/name changed");
        assert_eq!(want.kind, got.kind, "case {} kind changed", got.name);
        assert_eq!(want.bytes, got.bytes, "case {} byte encoding changed — this breaks wire compatibility!", got.name);
    }
}

/// 额外做一次真实的 socket 字节流交换（in-process），保证 IO 层与编解码层配合正确。
#[test]
fn hello_exchange_over_tcp_pair() {
    use std::io::Write as _;
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        let hello = read_hello(&mut sock).unwrap();
        assert_eq!(hello.device_name, "Pixel 8");
        let ack = hello_ack(AckStatus::Ok);
        send_hello_ack(&mut sock, &ack).unwrap();
        sock.flush().unwrap();
    });

    let mut client = std::net::TcpStream::connect(addr).unwrap();
    send_hello(&mut client, &hello_basic()).unwrap();
    let got = read_hello_ack(&mut client).unwrap();
    assert_eq!(got.status, AckStatus::Ok);
    assert_eq!(got.device_name, "DESKTOP-PC");
    server.join().unwrap();
}
