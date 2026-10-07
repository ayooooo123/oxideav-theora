//! The registry decoder reports the picture region of the identification
//! header in force (oxideav-core `Decoder::output_video_dimensions` /
//! `output_pixel_format`): every frame is cropped to it (§2.2), and
//! headers sent once the decoder is live are ignored, so it describes
//! every frame. The streams come from the crate's own encoder.
//!
//! An odd picture offset with an even picture size along a subsampled
//! axis gives the §4.4.4 chroma crop one more sample than the core
//! layout has: such a stream reports its size and no format.

use oxideav_core::{
    CodecId, CodecParameters, Decoder, Encoder as _, Frame, Packet, PixelFormat as Core, TimeBase,
    VideoFrame, VideoPlane,
};
use oxideav_theora::{
    PixelFormat, SetupHeaderTables, TheoraEncoder, TheoraIdentHeader, THEORA_CODEC_ID,
};

/// The header and data packets of `frames` 4:2:0 frames of `w`×`h`.
fn stream(w: u32, h: u32, frames: usize) -> Vec<Packet> {
    let ident = TheoraIdentHeader::for_picture(w, h, PixelFormat::Yuv420, 25, 1).unwrap();
    let mut enc = TheoraEncoder::with_keyframe_interval(
        CodecId::new(THEORA_CODEC_ID),
        ident,
        SetupHeaderTables::encoder_defaults(),
        40,
        30,
    )
    .unwrap();
    let plane = |pw: u32, ph: u32, seed: usize| VideoPlane {
        stride: pw as usize,
        data: (0..(pw * ph) as usize)
            .map(|i| ((i * 5 + seed * 11) % 256) as u8)
            .collect(),
    };
    for t in 0..frames {
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let frame = VideoFrame {
            pts: Some(t as i64),
            planes: vec![plane(w, h, t), plane(cw, ch, t + 1), plane(cw, ch, t + 2)],
        };
        enc.send_frame(&Frame::Video(frame)).unwrap();
    }
    enc.flush().unwrap();
    (0..).map_while(|_| enc.receive_packet().ok()).collect()
}

fn decoder() -> Box<dyn Decoder> {
    oxideav_theora::make_decoder(&CodecParameters::video(CodecId::new(THEORA_CODEC_ID)))
        .expect("decoder")
}

fn report(dec: &dyn Decoder) -> (Option<(u32, u32)>, Option<Core>) {
    (dec.output_video_dimensions(), dec.output_pixel_format())
}

#[test]
fn every_frame_has_the_reported_picture_region() {
    for (w, h) in [(33, 17), (48, 32)] {
        let packets = stream(w, h, 3);
        let mut dec = decoder();
        let mut frames = 0;
        for p in &packets {
            dec.send_packet(p).expect("send");
            if !p.flags.header {
                assert_eq!(
                    report(&*dec),
                    (Some((w, h)), Some(Core::Yuv420P)),
                    "{w}x{h}: before frame {frames}"
                );
            }
            while let Ok(Frame::Video(frame)) = dec.receive_frame() {
                assert_eq!(
                    report(&*dec),
                    (Some((w, h)), Some(Core::Yuv420P)),
                    "{w}x{h}: frame {frames}"
                );
                let planes = frame.image_planes();
                assert_eq!(planes.len(), 3, "{w}x{h}: planes");
                for (i, plane) in planes.iter().enumerate() {
                    let (pw, ph) = Core::Yuv420P.plane_dimensions(i, w, h).unwrap();
                    assert_eq!(
                        plane.stride, pw as usize,
                        "{w}x{h} frame {frames}: plane {i} stride"
                    );
                    assert_eq!(
                        plane.data.len(),
                        (pw * ph) as usize,
                        "{w}x{h} frame {frames}: plane {i} rows"
                    );
                }
                frames += 1;
            }
        }
        assert_eq!(frames, 3, "{w}x{h}: frames");
    }
}

/// A 48×32 frame whose 32-wide picture starts at column 1: chroma
/// columns 0..17 cover it, where `Yuv420P` at 32 wide has 16.
#[test]
fn odd_offset_even_width_reports_no_format() {
    let ident = stream(48, 32, 1)
        .into_iter()
        .find(|p| p.flags.header && p.data[0] == 0x80)
        .expect("identification header");
    let mut data = ident.data;
    // §6.2: PICW (24 bits) at bytes 14..17, PICX at byte 20.
    data[14..17].copy_from_slice(&[0, 0, 32]);
    data[20] = 1;
    let mut dec = decoder();
    dec.send_packet(&Packet::new(0, TimeBase::new(1, 25), data))
        .expect("identification header");
    assert_eq!(report(&*dec), (Some((32, 32)), None));
}
