use rairstream::session::ModernAirPlaySession;

#[test]
fn airplay2_transport_name_remains_stable() {
    assert_eq!(ModernAirPlaySession::transport_name(), "airplay2");
}
