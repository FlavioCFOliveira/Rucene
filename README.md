# Rucene
Rucene is a port of Apache Lucene Core to rust.
This port targets only Apache Lucene Core 10.5.0 

## Crate Library (rust)
This project is a Rust Crate that ports Apache Lucene Core to a crate libray. 

## Port parity
As long as it is possible this project pretends to provide absolute partity with Apache Lucene Core on two dimensions:

 1. **Functional Parity** - Same functionality, same organization just different language. (better performance, better memory management)
 2. **100% Index Compatibility** - this crate should **Read and Write index files** 100% compatible with Apach Lucene Core 10.5.0
