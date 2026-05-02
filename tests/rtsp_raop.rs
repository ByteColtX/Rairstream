use rairstream::rtsp::raop::RaopSession;

#[test]
fn raop_transport_name_remains_stable() {
    assert_eq!(RaopSession::transport_name(), "raop");
}
