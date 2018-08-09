#![allow(non_camel_case_types)]

use std::ptr;
include!(concat!(env!("OUT_DIR"), "/dns.rs"));

use grains::protocol::ip::to_ipv4;
use grains::*;

pub struct DNS;

impl EBPFGrain<'static> for DNS {
    fn code() -> &'static [u8] {
        include_bytes!(concat!(env!("OUT_DIR"), "/dns.elf"))
    }

    fn get_handler(&self, _id: &str) -> EventCallback {
        Box::new(|raw| {
            let query = DNSQuery::from(_data_dns_query::from(raw));
            let tags = query.to_tags();

            Some(Message::Single(Measurement::new(
                COUNTER | HISTOGRAM | METER,
                "dns.answer".to_string(),
                Unit::Count(1),
                tags,
            )))
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct DNSQuery {
    pub id: u16,
    pub destination_ip: Ipv4Addr,
    pub destination_port: u16,
    pub source_ip: Ipv4Addr,
    pub source_port: u16,
    pub address: String,
    pub qtype: u16,
    pub qclass: u16,
}

impl From<_data_dns_query> for DNSQuery {
    fn from(data: _data_dns_query) -> DNSQuery {
        DNSQuery {
            id: to_le(data.id),
            destination_ip: to_ipv4(data.daddr),
            source_ip: to_ipv4(data.saddr),
            destination_port: to_le(data.dport),
            source_port: to_le(data.sport),
            address: from_dns_prefix_labels(unsafe {
                &*(&data.address as *const [i8] as *const [u8])
            }),
            qtype: to_le(data.qtype),
            qclass: to_le(data.qclass),
        }
    }
}

impl ToTags for DNSQuery {
    fn to_tags(self) -> Tags {
        let mut tags = Tags::new();

        tags.insert("q_type", self.qclass.to_string());
        tags.insert("q_class", self.qclass.to_string());
        tags.insert("q_addr", self.address.to_string());
        tags.insert("q_id", self.id.to_string());

        tags.insert("d_ip", self.destination_ip.to_string());
        tags.insert("d_port", self.destination_port.to_string());

        tags.insert("s_ip", self.source_ip.to_string());
        tags.insert("s_port", self.source_port.to_string());

        tags
    }
}

pub fn from_dns_prefix_labels(address: &[u8]) -> String {
    let mut ret = String::new();
    let mut i = 0usize;

    while i < address.len() {
        let label_len = address[i] as usize;
        if label_len == 0 {
            break;
        }
        i += 1;

        let label = String::from_utf8_lossy(&address[i..(i + label_len)]);
        ret.push_str(&label);
        ret.push('.');
        i += label_len;
    }

    ret
}

mod test {
    #[test]
    fn parse_dns_labels() {
        use dns::from_dns_prefix_labels;
        assert_eq!(
            from_dns_prefix_labels(b"\x04asdf\x03com\x00"),
            String::from("asdf.com.")
        );
        assert_eq!(
            from_dns_prefix_labels(b"\x051e100\x03com\x00"),
            String::from("1e100.com.")
        );
        assert_eq!(
            from_dns_prefix_labels(b"\x05\x01e100\x03com\x00"),
            String::from("\x01e100.com.")
        );
    }
}
