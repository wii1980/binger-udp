pub(crate) fn try_send_batch(
    _fd: std::os::fd::RawFd,
    _batch: &crate::batch::SendBatchRaw,
) -> std::io::Result<usize> {
    unreachable!("linux platform backend selected, but sendmmsg stub used")
}

pub(crate) fn try_recv_batch(
    _fd: std::os::fd::RawFd,
    _batch: &mut crate::batch::RecvBatchRaw,
) -> std::io::Result<usize> {
    unreachable!("linux platform backend selected, but recvmmsg stub used")
}
