# ringu

A simple rust no_std ring buffer.
The intent is to be thread-safe, possibly multi-producer, multi-consumer. 
This is work in progress and YMMV.

## Example

```rust
    let mut buf = Ringu::default();
    let mut push_count = 0;
    for i in 0..128 {
        push_count += buf.push_one(i as u8);
    }
    assert_eq!(push_count, 128);
    let mut read_count = 0;
    for i in 0..128) {
        let (nread, _val) = buf.read_one();
        read_count += nread;
    }   
    assert_eq!(read_count, 128);
```

## License

BSD-3:  See LICENSE file

## Status

 - [x] Functional testing (in progress)
 - [ ] Tested on cortex-m4
 - [x] Example code (see README)
 - [ ] Generic variable length buffer
 - [ ] CI
