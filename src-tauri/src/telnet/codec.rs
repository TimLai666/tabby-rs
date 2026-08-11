use std::collections::HashSet;

pub const IAC: u8 = 255;
pub const SE: u8 = 240;
pub const NOP: u8 = 241;
pub const SB: u8 = 250;
pub const WILL: u8 = 251;
pub const WONT: u8 = 252;
pub const DO: u8 = 253;
pub const DONT: u8 = 254;
pub const ECHO: u8 = 1;
pub const SUPPRESS_GO_AHEAD: u8 = 3;
pub const TERMINAL_TYPE: u8 = 24;
pub const NAWS: u8 = 31;
pub const SUBOPTION_IS: u8 = 0;
pub const SUBOPTION_SEND: u8 = 1;
pub const MAX_SUBNEGOTIATION_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TelnetEvent {
    Data(Vec<u8>),
    Command { command: u8, option: Option<u8> },
    Subnegotiation { option: u8, payload: Vec<u8> },
    Malformed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecodeState {
    Data,
    Iac,
    Negotiation(u8),
    SubnegotiationOption,
    Subnegotiation { option: u8 },
    SubnegotiationIac { option: u8 },
}

impl Default for DecodeState {
    fn default() -> Self {
        Self::Data
    }
}

#[derive(Debug, Default)]
pub struct TelnetCodec {
    state: DecodeState,
    data: Vec<u8>,
    subnegotiation: Vec<u8>,
}

impl TelnetCodec {
    pub fn new() -> Self {
        Self {
            state: DecodeState::Data,
            ..Self::default()
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<TelnetEvent> {
        let mut events = Vec::new();
        for &byte in bytes {
            match self.state {
                DecodeState::Data if byte == IAC => {
                    self.flush_data(&mut events);
                    self.state = DecodeState::Iac;
                }
                DecodeState::Data => self.data.push(byte),
                DecodeState::Iac => match byte {
                    IAC => {
                        self.data.push(IAC);
                        self.state = DecodeState::Data;
                    }
                    SB => self.state = DecodeState::SubnegotiationOption,
                    WILL | WONT | DO | DONT => self.state = DecodeState::Negotiation(byte),
                    _ => {
                        events.push(TelnetEvent::Command {
                            command: byte,
                            option: None,
                        });
                        self.state = DecodeState::Data;
                    }
                },
                DecodeState::Negotiation(command) => {
                    events.push(TelnetEvent::Command {
                        command,
                        option: Some(byte),
                    });
                    self.state = DecodeState::Data;
                }
                DecodeState::SubnegotiationOption => {
                    self.subnegotiation.clear();
                    self.state = DecodeState::Subnegotiation { option: byte };
                }
                DecodeState::Subnegotiation { option } => {
                    if byte == IAC {
                        self.state = DecodeState::SubnegotiationIac { option };
                    } else if self.push_subnegotiation(byte) {
                        self.state = DecodeState::Data;
                        events.push(TelnetEvent::Malformed(
                            "subnegotiation payload exceeded 64 KiB".into(),
                        ));
                    }
                }
                DecodeState::SubnegotiationIac { option } => match byte {
                    IAC => {
                        if self.push_subnegotiation(IAC) {
                            self.state = DecodeState::Data;
                            events.push(TelnetEvent::Malformed(
                                "subnegotiation payload exceeded 64 KiB".into(),
                            ));
                        } else {
                            self.state = DecodeState::Subnegotiation { option };
                        }
                    }
                    SE => {
                        events.push(TelnetEvent::Subnegotiation {
                            option,
                            payload: std::mem::take(&mut self.subnegotiation),
                        });
                        self.state = DecodeState::Data;
                    }
                    _ => {
                        events.push(TelnetEvent::Malformed(
                            "invalid byte after IAC in subnegotiation".into(),
                        ));
                        self.state = DecodeState::Data;
                        if byte == IAC {
                            self.state = DecodeState::Iac;
                        } else {
                            self.data.push(byte);
                        }
                    }
                },
            }
        }
        self.flush_data(&mut events);
        events
    }

    fn push_subnegotiation(&mut self, byte: u8) -> bool {
        if self.subnegotiation.len() >= MAX_SUBNEGOTIATION_BYTES {
            return true;
        }
        self.subnegotiation.push(byte);
        false
    }

    fn flush_data(&mut self, events: &mut Vec<TelnetEvent>) {
        if !self.data.is_empty() {
            events.push(TelnetEvent::Data(std::mem::take(&mut self.data)));
        }
    }
}

pub fn encode_command(command: u8, option: u8) -> Vec<u8> {
    vec![IAC, command, option]
}

pub fn encode_subnegotiation(option: u8, payload: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(payload.len() + 5);
    encoded.extend_from_slice(&[IAC, SB, option]);
    for &byte in payload {
        encoded.push(byte);
        if byte == IAC {
            encoded.push(IAC);
        }
    }
    encoded.extend_from_slice(&[IAC, SE]);
    encoded
}

#[derive(Debug)]
pub struct TelnetNegotiator {
    terminal_type: Vec<u8>,
    local_echo: bool,
    columns: u16,
    rows: u16,
    local_options: HashSet<u8>,
    remote_options: HashSet<u8>,
    rejected_local_options: HashSet<u8>,
    force_echo: bool,
}

impl TelnetNegotiator {
    pub fn new(terminal_type: String, local_echo: bool) -> Self {
        let terminal_type = if terminal_type.is_empty() {
            "xterm-256color".into()
        } else {
            terminal_type.bytes().filter(|byte| *byte != IAC).collect()
        };
        Self {
            terminal_type,
            local_echo,
            columns: 80,
            rows: 30,
            local_options: HashSet::new(),
            remote_options: HashSet::new(),
            rejected_local_options: HashSet::new(),
            force_echo: false,
        }
    }

    pub fn initial_requests(&mut self) -> Vec<Vec<u8>> {
        vec![
            encode_command(DO, SUPPRESS_GO_AHEAD),
            encode_command(WILL, TERMINAL_TYPE),
            encode_command(WILL, NAWS),
        ]
    }

    pub fn resize(&mut self, columns: u16, rows: u16) -> Vec<u8> {
        if columns > 0 {
            self.columns = columns;
        }
        if rows > 0 {
            self.rows = rows;
        }
        self.naws()
    }

    pub fn force_echo(&self) -> bool {
        self.force_echo
    }

    pub fn handle(&mut self, event: &TelnetEvent) -> Vec<Vec<u8>> {
        match event {
            TelnetEvent::Command {
                command,
                option: Some(option),
            } => self.handle_command(*command, *option),
            TelnetEvent::Subnegotiation { option, payload }
                if *option == TERMINAL_TYPE && payload.first() == Some(&SUBOPTION_SEND) =>
            {
                let mut value = vec![SUBOPTION_IS];
                value.extend_from_slice(&self.terminal_type);
                vec![encode_subnegotiation(TERMINAL_TYPE, &value)]
            }
            _ => Vec::new(),
        }
    }

    fn handle_command(&mut self, command: u8, option: u8) -> Vec<Vec<u8>> {
        match command {
            WILL => {
                if self.remote_options.insert(option) {
                    if matches!(option, ECHO | SUPPRESS_GO_AHEAD) {
                        let mut responses = vec![encode_command(DO, option)];
                        if option == ECHO && self.local_echo {
                            responses.push(encode_command(WONT, ECHO));
                        }
                        responses
                    } else {
                        vec![encode_command(DONT, option)]
                    }
                } else {
                    Vec::new()
                }
            }
            WONT => {
                if self.remote_options.remove(&option) {
                    vec![encode_command(DONT, option)]
                } else {
                    Vec::new()
                }
            }
            DO => {
                if matches!(option, ECHO | TERMINAL_TYPE | NAWS) {
                    self.rejected_local_options.remove(&option);
                    let was_enabled = self.local_options.insert(option);
                    if option == ECHO {
                        self.force_echo = true;
                    }
                    let mut responses = if was_enabled {
                        vec![encode_command(WILL, option)]
                    } else {
                        Vec::new()
                    };
                    if option == NAWS {
                        responses.push(self.naws());
                    }
                    responses
                } else {
                    if self.rejected_local_options.insert(option) {
                        vec![encode_command(WONT, option)]
                    } else {
                        Vec::new()
                    }
                }
            }
            DONT => {
                if self.local_options.remove(&option) {
                    if option == ECHO {
                        self.force_echo = false;
                    }
                    vec![encode_command(WONT, option)]
                } else {
                    self.rejected_local_options.remove(&option);
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    fn naws(&self) -> Vec<u8> {
        encode_subnegotiation(
            NAWS,
            &[
                (self.columns >> 8) as u8,
                self.columns as u8,
                (self.rows >> 8) as u8,
                self.rows as u8,
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_data_and_literal_iac_across_every_byte_boundary() {
        let input = [b'a', IAC, IAC, b'b', IAC, WILL, ECHO, b'c'];
        for split in 0..=input.len() {
            let mut codec = TelnetCodec::new();
            let mut events = codec.feed(&input[..split]);
            events.extend(codec.feed(&input[split..]));
            let data = events
                .iter()
                .filter_map(|event| match event {
                    TelnetEvent::Data(data) => Some(data.as_slice()),
                    _ => None,
                })
                .flatten()
                .copied()
                .collect::<Vec<_>>();
            let commands = events
                .iter()
                .filter_map(|event| match event {
                    TelnetEvent::Command { command, option } => Some((*command, *option)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(data, vec![b'a', IAC, b'b', b'c'], "split at {split}");
            assert_eq!(commands, vec![(WILL, Some(ECHO))], "split at {split}");
        }
    }

    #[test]
    fn decodes_escaped_subnegotiation_and_respects_chunk_boundaries() {
        let input = encode_subnegotiation(NAWS, &[0, 80, IAC, 30]);
        let mut codec = TelnetCodec::new();
        let mut events = Vec::new();
        for byte in input {
            events.extend(codec.feed(&[byte]));
        }
        assert_eq!(
            events,
            vec![TelnetEvent::Subnegotiation {
                option: NAWS,
                payload: vec![0, 80, IAC, 30],
            }]
        );
    }

    #[test]
    fn malformed_subnegotiation_is_bounded_and_reported() {
        let mut codec = TelnetCodec::new();
        let mut input = vec![IAC, SB, NAWS];
        input.resize(MAX_SUBNEGOTIATION_BYTES + 4, b'x');
        let events = codec.feed(&input);
        assert!(events
            .iter()
            .any(|event| matches!(event, TelnetEvent::Malformed(_))));
    }

    #[test]
    fn negotiates_supported_options_without_repeating_replies() {
        let mut negotiator = TelnetNegotiator::new("xterm-256color".into(), true);
        assert_eq!(
            negotiator.initial_requests(),
            vec![
                encode_command(DO, SUPPRESS_GO_AHEAD),
                encode_command(WILL, TERMINAL_TYPE),
                encode_command(WILL, NAWS),
            ]
        );
        assert_eq!(
            negotiator.handle(&TelnetEvent::Command {
                command: WILL,
                option: Some(ECHO),
            }),
            vec![encode_command(DO, ECHO), encode_command(WONT, ECHO)]
        );
        assert!(!negotiator.force_echo());
        assert!(negotiator
            .handle(&TelnetEvent::Command {
                command: WILL,
                option: Some(ECHO),
            })
            .is_empty());
        assert_eq!(
            negotiator.handle(&TelnetEvent::Command {
                command: DO,
                option: Some(NAWS),
            }),
            vec![
                encode_command(WILL, NAWS),
                encode_subnegotiation(NAWS, &[0, 80, 0, 30]),
            ]
        );
        assert_eq!(
            negotiator.handle(&TelnetEvent::Command {
                command: DO,
                option: Some(42),
            }),
            vec![encode_command(WONT, 42)]
        );
        assert!(negotiator
            .handle(&TelnetEvent::Command {
                command: DO,
                option: Some(42),
            })
            .is_empty());
    }

    #[test]
    fn terminal_type_and_naws_are_encoded_in_network_byte_order() {
        let mut negotiator = TelnetNegotiator::new("vt100".into(), false);
        assert_eq!(
            negotiator.handle(&TelnetEvent::Subnegotiation {
                option: TERMINAL_TYPE,
                payload: vec![SUBOPTION_SEND],
            }),
            vec![encode_subnegotiation(
                TERMINAL_TYPE,
                &[SUBOPTION_IS, b'v', b't', b'1', b'0', b'0']
            )]
        );
        assert_eq!(
            negotiator.resize(300, 120),
            encode_subnegotiation(NAWS, &[1, 44, 0, 120])
        );
    }
}
