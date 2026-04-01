use devx::ports::PortAllocator;
use std::net::TcpListener;

#[test]
fn test_allocate_free_port_gets_preferred() {
    let mut allocator = PortAllocator::new();
    let alloc = allocator.allocate("svc", 59123).unwrap();
    // May get remapped if port is already in use on CI, so just check it's valid
    assert!(alloc.actual > 0);
    assert_eq!(alloc.preferred, Some(59123));
    if !alloc.remapped {
        assert_eq!(alloc.actual, 59123);
    }
}

#[test]
fn test_allocate_busy_port_gets_free() {
    // Bind a port first to make it busy
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let busy_port = listener.local_addr().unwrap().port();

    let mut allocator = PortAllocator::new();
    let alloc = allocator.allocate("svc", busy_port).unwrap();
    assert!(alloc.remapped);
    assert_ne!(alloc.actual, busy_port);
    assert_eq!(alloc.preferred, Some(busy_port));
    assert!(alloc.actual > 0);
}

#[test]
fn test_allocate_no_preferred() {
    let mut allocator = PortAllocator::new();
    let alloc = allocator.allocate_any("svc").unwrap();
    assert!(alloc.actual > 0);
    assert_eq!(alloc.preferred, None);
    assert!(!alloc.remapped);
}
