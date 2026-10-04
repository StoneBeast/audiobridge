//! 会话层端到端测试：ToneSource -> (网络) -> NullOutput，全内存/localhost。

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use audiobridge_core::session::{
    run_sender, spawn_receiver, NullOutput, ReceiveOptions, SenderStats, ReceiverStats,
    SendOptions, ToneSource,
};

#[test]
fn sender_to_receiver_over_localhost() {
    let port = pick_port();
    let rx_stats = Arc::new(Mutex::new(ReceiverStats::default()));
    let tx_stats = Arc::new(Mutex::new(SenderStats::default()));
    let stop = Arc::new(AtomicBool::new(false));

    // 接收端
    let rx_stop = Arc::clone(&stop);
    let rx_stats2 = Arc::clone(&rx_stats);
    let handles = spawn_receiver(
        ReceiveOptions {
            port,
            device_name: "TEST-PC".into(),
            frame_ms: 20,
            target_ms: 60,
            max_ms: 250,
            token: None,
            volume_permille: Arc::new(AtomicU32::new(1000)),
        },
        Arc::clone(&rx_stats),
        Arc::clone(&stop),
        move || Ok(Box::new(NullOutput::new(20)) as Box<dyn audiobridge_core::session::AudioOutput>),
    );
    let _ = handles;

    // 等监听就绪
    std::thread::sleep(Duration::from_millis(200));

    // 发送端（在自己的线程里跑 2 秒后停）
    let tx_stop = Arc::new(AtomicBool::new(false));
    let sender_thread = {
        let tx_stats = Arc::clone(&tx_stats);
        let stop = Arc::clone(&tx_stop);
        std::thread::spawn(move || {
            let mut tone = ToneSource::new(440.0, 48_000, 2, 20);
            run_sender(
                &SendOptions {
                    host: "127.0.0.1".into(),
                    port,
                    device_name: "TEST-PHONE".into(),
                    frame_ms: 20,
                    token: None,
                },
                &mut tone,
                tx_stats,
                stop,
            )
        })
    };

    // 收 1.5 秒数据后停发送端
    std::thread::sleep(Duration::from_millis(1500));
    tx_stop.store(true, Ordering::Relaxed);
    let send_result = sender_thread.join().unwrap();
    assert!(send_result.is_ok(), "sender failed: {send_result:?}");

    let sent = tx_stats.lock().unwrap().sent_bytes;
    let (pushed, underruns) = {
        let s = rx_stats2.lock().unwrap();
        (s.pushed_bytes, s.underruns)
    };
    eprintln!("sent={sent} pushed={pushed} underruns={underruns}");
    // 1.5 秒 48k/2ch/s16 ≈ 288KB；宽裕下限
    assert!(sent > 150_000, "sent too little: {sent}");
    assert!(pushed > 150_000, "received too little: {pushed}");

    // 停接收端
    stop.store(true, Ordering::Relaxed);
}

#[test]
fn busy_second_connection_rejected() {
    let port = pick_port();
    let stop = Arc::new(AtomicBool::new(false));
    spawn_receiver(
        ReceiveOptions {
            port,
            ..Default::default()
        },
        Arc::new(Mutex::new(ReceiverStats::default())),
        Arc::clone(&stop),
        move || Ok(Box::new(NullOutput::new(20)) as Box<dyn audiobridge_core::session::AudioOutput>),
    );
    std::thread::sleep(Duration::from_millis(200));

    // 连接 1：握手后保持连接（不发送数据）——用裸 TCP 手工握手
    use audiobridge_core::framing::send_hello;
    use std::io::Write;
    let c1 = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut w1 = c1.try_clone().unwrap();
    send_hello(
        &mut w1,
        &audiobridge_core::Hello {
            device_name: "HOLD".into(),
            codec: audiobridge_core::Codec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            frame_ms: 20,
            buffer_target_ms: 80,
            auth_token_sha256: [0; 32],
        },
    )
    .unwrap();
    w1.flush().unwrap();
    std::thread::sleep(Duration::from_millis(200));

    // 连接 2：应收到 BUSY
    let mut c2 = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut w2 = c2.try_clone().unwrap();
    send_hello(
        &mut w2,
        &audiobridge_core::Hello {
            device_name: "SECOND".into(),
            codec: audiobridge_core::Codec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            frame_ms: 20,
            buffer_target_ms: 80,
            auth_token_sha256: [0; 32],
        },
    )
    .unwrap();
    w2.flush().unwrap();
    let ack = audiobridge_core::framing::read_hello_ack_raw(&mut c2).unwrap();
    assert_eq!(ack.status, audiobridge_core::AckStatus::Busy);

    stop.store(true, Ordering::Relaxed);
}

fn pick_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
