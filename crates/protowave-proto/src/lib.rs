//! ProtoWave wire protocol types, generated from `proto/` (PRD §8.1).

pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/protowave.v1.rs"));

    use prost::Message;

    impl Envelope {
        pub fn new(channel: Channel, payload: Vec<u8>) -> Self {
            Self {
                channel: channel as i32,
                payload,
            }
        }

        /// Wrap a control-channel message (e.g. auth) into an envelope.
        pub fn control(msg: &impl Message) -> Self {
            Self::new(Channel::Control, msg.encode_to_vec())
        }

        pub fn encode_frame(&self) -> Vec<u8> {
            self.encode_to_vec()
        }

        pub fn decode_frame(bytes: &[u8]) -> Result<Self, prost::DecodeError> {
            Self::decode(bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::v1::{control_message, Channel, ControlMessage, Envelope, Subscribe};
    use prost::Message;

    #[test]
    fn envelope_roundtrip() {
        let control = ControlMessage {
            kind: Some(control_message::Kind::Subscribe(Subscribe {
                wavelet: "example.org/w+1/conv+root".into(),
                state_vector: vec![0],
            })),
        };
        let env = Envelope::control(&control);
        let bytes = env.encode_frame();
        let back = Envelope::decode_frame(&bytes).unwrap();
        assert_eq!(back.channel, Channel::Control as i32);
        let control_back = ControlMessage::decode(back.payload.as_slice()).unwrap();
        match control_back.kind {
            Some(control_message::Kind::Subscribe(s)) => {
                assert_eq!(s.wavelet, "example.org/w+1/conv+root");
            }
            other => panic!("unexpected kind: {other:?}"),
        }
    }
}
