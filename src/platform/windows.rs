pub(crate) fn try_send_batch(
    _fd: std::os::fd::RawFd,
    _batch: &crate::batch::SendBatchRaw,
) -> std::io::Result<usize> {
    unreachable!("Windows platform backend not yet implemented")
}

pub(crate) fn try_recv_batch(
    _fd: std::os::fd::RawFd,
    _batch: &mut crate::batch::RecvBatchRaw,
) -> std::io::Result<usize> {
    unreachable!("Windows platform backend not yet implemented")
}
