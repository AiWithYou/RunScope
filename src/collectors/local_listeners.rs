use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::bail;

use crate::model::ListeningEndpoint;

#[cfg(windows)]
pub fn collect_local_listeners() -> anyhow::Result<HashMap<u32, Vec<ListeningEndpoint>>> {
    let mut listeners = HashMap::new();
    collect_ipv4(&mut listeners)?;
    collect_ipv6(&mut listeners)?;

    for endpoints in listeners.values_mut() {
        endpoints.sort_by(|left, right| {
            left.port
                .cmp(&right.port)
                .then_with(|| left.url.cmp(&right.url))
                .then_with(|| left.bind_address.cmp(&right.bind_address))
        });
        endpoints.dedup_by(|left, right| {
            left.port == right.port
                && left.url == right.url
                && left.bind_address == right.bind_address
        });
    }

    Ok(listeners)
}

#[cfg(not(windows))]
pub fn collect_local_listeners() -> anyhow::Result<HashMap<u32, Vec<ListeningEndpoint>>> {
    Ok(HashMap::new())
}

#[cfg(windows)]
fn collect_ipv4(listeners: &mut HashMap<u32, Vec<ListeningEndpoint>>) -> anyhow::Result<()> {
    use windows::Win32::NetworkManagement::IpHelper::{
        MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows::Win32::Networking::WinSock::AF_INET;

    let buffer = tcp_table(AF_INET.0 as u32, TCP_TABLE_OWNER_PID_LISTENER)?;
    for row in tcp_rows::<MIB_TCPROW_OWNER_PID>(&buffer, "IPv4")? {
        let pid = row.dwOwningPid;
        let port = port_from_network_order(row.dwLocalPort);
        if pid == 0 || port == 0 {
            continue;
        }
        let address = Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes()).to_string();
        listeners
            .entry(pid)
            .or_default()
            .push(ListeningEndpoint::new(address, port, false));
    }
    Ok(())
}

#[cfg(windows)]
fn collect_ipv6(listeners: &mut HashMap<u32, Vec<ListeningEndpoint>>) -> anyhow::Result<()> {
    use windows::Win32::NetworkManagement::IpHelper::{
        MIB_TCP6ROW_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows::Win32::Networking::WinSock::AF_INET6;

    let buffer = tcp_table(AF_INET6.0 as u32, TCP_TABLE_OWNER_PID_LISTENER)?;
    for row in tcp_rows::<MIB_TCP6ROW_OWNER_PID>(&buffer, "IPv6")? {
        let pid = row.dwOwningPid;
        let port = port_from_network_order(row.dwLocalPort);
        if pid == 0 || port == 0 {
            continue;
        }
        let address = Ipv6Addr::from(row.ucLocalAddr).to_string();
        listeners
            .entry(pid)
            .or_default()
            .push(ListeningEndpoint::new(address, port, true));
    }

    Ok(())
}

#[cfg(windows)]
fn tcp_rows<T: Copy>(buffer: &[u8], label: &str) -> anyhow::Result<Vec<T>> {
    let header_size = std::mem::size_of::<u32>();
    if buffer.len() < header_size {
        bail!("{label} TCP table buffer is too small");
    }

    let count = u32::from_ne_bytes(
        buffer[..header_size]
            .try_into()
            .expect("u32 header has a fixed size"),
    ) as usize;
    let rows_size = count
        .checked_mul(std::mem::size_of::<T>())
        .and_then(|size| size.checked_add(header_size))
        .ok_or_else(|| anyhow::anyhow!("{label} TCP table row count overflowed"))?;
    if rows_size > buffer.len() {
        bail!(
            "{label} TCP table buffer is truncated: expected {rows_size} bytes, got {}",
            buffer.len()
        );
    }

    let mut rows = Vec::with_capacity(count);
    for index in 0..count {
        let offset = header_size + index * std::mem::size_of::<T>();
        let row = unsafe { std::ptr::read_unaligned(buffer.as_ptr().add(offset).cast::<T>()) };
        rows.push(row);
    }
    Ok(rows)
}

#[cfg(windows)]
fn tcp_table(
    address_family: u32,
    table_class: windows::Win32::NetworkManagement::IpHelper::TCP_TABLE_CLASS,
) -> anyhow::Result<Vec<u8>> {
    use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
    use windows::Win32::NetworkManagement::IpHelper::GetExtendedTcpTable;

    let mut size = 0;
    let first_status =
        unsafe { GetExtendedTcpTable(None, &mut size, false, address_family, table_class, 0) };

    if first_status != ERROR_INSUFFICIENT_BUFFER.0 && first_status != NO_ERROR.0 {
        bail!("GetExtendedTcpTable size query failed with code {first_status}");
    }
    if size == 0 {
        return Ok(Vec::new());
    }

    let mut buffer = Vec::new();
    for _ in 0..3 {
        buffer.resize(size as usize, 0);
        let status = unsafe {
            GetExtendedTcpTable(
                Some(buffer.as_mut_ptr().cast()),
                &mut size,
                false,
                address_family,
                table_class,
                0,
            )
        };
        if status == NO_ERROR.0 {
            buffer.truncate(size as usize);
            return Ok(buffer);
        }
        if status != ERROR_INSUFFICIENT_BUFFER.0 {
            bail!("GetExtendedTcpTable failed with code {status}");
        }
    }
    bail!("GetExtendedTcpTable kept resizing during collection")
}

fn port_from_network_order(port: u32) -> u16 {
    u16::from_be(port as u16)
}

#[cfg(test)]
mod tests {
    use super::port_from_network_order;
    #[cfg(windows)]
    use super::tcp_rows;

    #[test]
    fn decodes_network_order_port() {
        assert_eq!(port_from_network_order(0x901f), 8080);
    }

    #[cfg(windows)]
    #[test]
    fn parses_unaligned_rows_and_rejects_truncated_buffers() {
        #[repr(C)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        struct Row {
            first: u16,
            second: u16,
        }

        let mut valid = 1_u32.to_ne_bytes().to_vec();
        valid.extend_from_slice(&7_u16.to_ne_bytes());
        valid.extend_from_slice(&9_u16.to_ne_bytes());
        assert_eq!(
            tcp_rows::<Row>(&valid, "test").unwrap(),
            vec![Row {
                first: 7,
                second: 9
            }]
        );

        valid.pop();
        assert!(tcp_rows::<Row>(&valid, "test").is_err());
    }
}
