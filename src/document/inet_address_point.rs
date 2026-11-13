//! An indexed 128-bit IP address field, ported from
//! `org.apache.lucene.document.InetAddressPoint` (Lucene 10.5.0,
//! `lucene/core/src/java/org/apache/lucene/document/InetAddressPoint.java`).
//!
//! Finding all documents within a range at search time is efficient. Multiple
//! values for the same field in one document are allowed.
//!
//! The field supports both IPv4 and IPv6 addresses: an IPv4 address is
//! converted to an [IPv4-mapped IPv6 address](https://tools.ietf.org/html/rfc4291#section-2.5.5),
//! so indexing `1.2.3.4` is the same as indexing `::FFFF:1.2.3.4`.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

use crate::analysis::{Analyzer, TokenStream};
use crate::document::{
    fields_data_simple_name, FieldData, FieldType, FieldValueSetters, InvertableType, NumericValue,
    PointValueStream, StoredValue,
};
use crate::error::{LuceneError, LuceneErrorKind, Result};
use crate::index::{IndexableField, IndexableFieldType};
use crate::search::point_in_set_query::{PointFormatter, PointInSetQuery};
use crate::search::point_range_query::{DimensionFormatter, PointRangeQuery};
use crate::search::Query;
use crate::util::atomic_ref_cell::{AtomicRef, AtomicRefCell};
use crate::util::{BytesRef, NumericUtils};

/// An indexed 128-bit IP address field.
///
/// Equivalent to `org.apache.lucene.document.InetAddressPoint`
/// (`InetAddressPoint.java:51-306`).
///
/// **Representation of `java.net.InetAddress`.** Every Java method of this
/// class takes or returns a `java.net.InetAddress`; this port takes and
/// returns [`std::net::IpAddr`], its Rust equivalent. The two differ in one
/// place, handled where an address enters this type: Java never holds an
/// IPv4-mapped address (`::ffff:a.b.c.d`) as an `Inet6Address` —
/// `InetAddress.getByName` and `InetAddress.getByAddress` both return the
/// `Inet4Address` `a.b.c.d` for it — while an [`IpAddr::V6`] can hold one. This
/// port therefore reads an IPv4-mapped [`IpAddr::V6`] as the `Inet4Address`
/// Java would hold (see [`get_address`]), so `getAddress()` is four bytes long,
/// as in Java, for the prefix-length check of
/// [`new_prefix_query`](Self::new_prefix_query) and for every rendering.
///
/// **Divergence from Lucene 10.5.0 (declared).** `Inet6Address` can carry a
/// scope id (`fe80::1%eth0`), which `getHostAddress()` appends after a `%`;
/// [`Ipv6Addr`] has no scope id, so no address here renders one. Every
/// address this class builds itself goes through `InetAddress.getByAddress`,
/// which sets no scope id, so only a caller-supplied scoped address — which
/// `std::net` cannot express — renders differently.
#[derive(Debug)]
pub struct InetAddressPoint {
    name: String,
    /// `InetAddressPoint.TYPE`, the one static object every Java instance
    /// references (`InetAddressPoint.java:60-68`).
    field_type: Arc<FieldType>,
    fields_data: AtomicRefCell<FieldData>,
}

impl InetAddressPoint {
    /// The number of bytes per dimension: 128 bits.
    ///
    /// Equivalent to `InetAddressPoint.BYTES` (`InetAddressPoint.java:57`).
    pub const BYTES: usize = 16;

    /// The RFC 4291 prefix that maps an IPv4 address into IPv6 space.
    ///
    /// Equivalent to the package-private `InetAddressPoint.IPV4_PREFIX`
    /// (`InetAddressPoint.java:60`).
    pub(crate) const IPV4_PREFIX: [u8; 12] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF];

    /// The minimum value that an ip address can hold: `::`.
    ///
    /// Equivalent to `InetAddressPoint.MIN_VALUE`, which is
    /// `decode(new byte[BYTES])` (`InetAddressPoint.java:71,76-77`).
    pub const MIN_VALUE: IpAddr = IpAddr::V6(Ipv6Addr::UNSPECIFIED);

    /// The maximum value that an ip address can hold: sixteen `0xFF` bytes.
    ///
    /// Equivalent to `InetAddressPoint.MAX_VALUE`, which is the decoding of
    /// sixteen `0xFF` bytes (`InetAddressPoint.java:74,78-80`).
    pub const MAX_VALUE: IpAddr = IpAddr::V6(Ipv6Addr::new(
        0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF,
    ));

    /// The one frozen type every `InetAddressPoint` shares: one dimension of
    /// [`BYTES`](Self::BYTES) bytes.
    ///
    /// Equivalent to the private static `InetAddressPoint.TYPE`
    /// (`InetAddressPoint.java:62-68`); every field holds this one object, as
    /// Java's fields do.
    fn field_type_static() -> &'static Arc<FieldType> {
        static TYPE: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
        TYPE.get_or_init(|| {
            let ft = FieldType::new();
            ft.set_dimensions(1, Self::BYTES as i32)
                .expect("INVARIANT: one dimension of sixteen bytes fits a new, mutable type");
            ft.freeze();
            Arc::new(ft)
        })
    }

    /// Creates a new `InetAddressPoint`, indexing the provided address.
    ///
    /// Equivalent to `InetAddressPoint(String, InetAddress)`
    /// (`InetAddressPoint.java:136-139`). Java also throws
    /// `IllegalArgumentException` for a null name or value; neither can be
    /// null here.
    pub fn new(name: impl Into<String>, point: IpAddr) -> Self {
        let field = Self {
            name: name.into(),
            field_type: Arc::clone(Self::field_type_static()),
            fields_data: AtomicRefCell::new(FieldData::Null),
        };
        field.set_inet_address_value(point);
        field
    }

    /// Returns the address that compares immediately greater than `address`.
    ///
    /// Equivalent to `InetAddressPoint.nextUp(InetAddress)`
    /// (`InetAddressPoint.java:89-99`): [`NumericUtils::add`] of one to the
    /// 16-byte encoding, decoded back.
    ///
    /// # Errors
    ///
    /// [`LuceneErrorKind::Arithmetic`] `"Overflow: there is no greater
    /// InetAddress than …"` when `address` is [`MAX_VALUE`](Self::MAX_VALUE),
    /// as Java throws `ArithmeticException`.
    pub fn next_up(address: IpAddr) -> Result<IpAddr> {
        if java_equals(address, Self::MAX_VALUE) {
            return Err(LuceneError::from(LuceneErrorKind::Arithmetic(format!(
                "Overflow: there is no greater InetAddress than {}",
                get_host_address(address)
            ))));
        }
        let mut delta = [0u8; Self::BYTES];
        delta[Self::BYTES - 1] = 1;
        let mut next_up_bytes = [0u8; Self::BYTES];
        NumericUtils::add(
            Self::BYTES,
            0,
            &Self::encode(address),
            &delta,
            &mut next_up_bytes,
        )?;
        Self::decode(&next_up_bytes)
    }

    /// Returns the address that compares immediately less than `address`.
    ///
    /// Equivalent to `InetAddressPoint.nextDown(InetAddress)`
    /// (`InetAddressPoint.java:107-117`): [`NumericUtils::subtract`] of one
    /// from the 16-byte encoding, decoded back.
    ///
    /// # Errors
    ///
    /// [`LuceneErrorKind::Arithmetic`] `"Underflow: there is no smaller
    /// InetAddress than …"` when `address` is [`MIN_VALUE`](Self::MIN_VALUE),
    /// as Java throws `ArithmeticException`.
    pub fn next_down(address: IpAddr) -> Result<IpAddr> {
        if java_equals(address, Self::MIN_VALUE) {
            return Err(LuceneError::from(LuceneErrorKind::Arithmetic(format!(
                "Underflow: there is no smaller InetAddress than {}",
                get_host_address(address)
            ))));
        }
        let mut delta = [0u8; Self::BYTES];
        delta[Self::BYTES - 1] = 1;
        let mut next_down_bytes = [0u8; Self::BYTES];
        NumericUtils::subtract(
            Self::BYTES,
            0,
            &Self::encode(address),
            &delta,
            &mut next_down_bytes,
        )?;
        Self::decode(&next_down_bytes)
    }

    /// Changes the value of this field.
    ///
    /// Equivalent to `InetAddressPoint.setInetAddressValue(InetAddress)`
    /// (`InetAddressPoint.java:120-125`). Java also throws
    /// `IllegalArgumentException("point must not be null")`; the value cannot
    /// be null here.
    pub fn set_inet_address_value(&self, value: IpAddr) {
        *self.fields_data.borrow_mut() = FieldData::Bytes(BytesRef::new(Self::encode(value)));
    }

    /// Returns the field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field's type, the shared `InetAddressPoint.TYPE`.
    pub fn field_type(&self) -> &FieldType {
        &self.field_type
    }

    /// Returns the encoded address.
    pub fn packed_value(&self) -> Option<AtomicRef<'_, [u8]>> {
        AtomicRef::filter_map(self.fields_data.borrow(), |data| match data {
            FieldData::Bytes(bytes) => Some(bytes.slice()),
            _ => None,
        })
        .ok()
    }

    /// Encodes an address into its 16-byte binary form.
    ///
    /// Equivalent to `InetAddressPoint.encode(InetAddress)`
    /// (`InetAddressPoint.java:170-182`): an IPv4 address is prefixed with
    /// [`IPV4_PREFIX`](Self::IPV4_PREFIX), an IPv6 address is its sixteen
    /// bytes. Java's `UnsupportedOperationException` for an address that is
    /// neither four nor sixteen bytes long cannot arise from an [`IpAddr`].
    pub fn encode(value: IpAddr) -> Vec<u8> {
        let address = get_address(value);
        if address.len() == 4 {
            let mut mapped = vec![0u8; Self::BYTES];
            mapped[..Self::IPV4_PREFIX.len()].copy_from_slice(&Self::IPV4_PREFIX);
            mapped[Self::IPV4_PREFIX.len()..].copy_from_slice(&address);
            mapped
        } else {
            address
        }
    }

    /// Decodes an address from its binary form.
    ///
    /// Equivalent to `InetAddressPoint.decode(byte[])`
    /// (`InetAddressPoint.java:185-192`), which returns
    /// `InetAddress.getByAddress(value)`: four bytes are an IPv4 address,
    /// sixteen bytes an IPv6 address — except an IPv4-mapped one, which
    /// `getByAddress` returns as the IPv4 address it maps.
    ///
    /// # Errors
    ///
    /// [`LuceneErrorKind::IllegalArgument`] `"encoded bytes are of incorrect
    /// length"` when `value` is neither four nor sixteen bytes long. Java
    /// chains the `UnknownHostException` that `getByAddress` threw as the
    /// cause; the port has no counterpart for that checked exception, so the
    /// error carries no cause (declared divergence, message and class exact).
    pub fn decode(value: &[u8]) -> Result<IpAddr> {
        match value.len() {
            4 => Ok(IpAddr::V4(Ipv4Addr::new(
                value[0], value[1], value[2], value[3],
            ))),
            16 => {
                let mut octets = [0u8; 16];
                octets.copy_from_slice(value);
                Ok(canonical(IpAddr::V6(Ipv6Addr::from(octets))))
            }
            _ => Err(LuceneError::illegal_argument(
                "encoded bytes are of incorrect length".to_string(),
            )),
        }
    }

    /// Creates a query matching one exact network address.
    ///
    /// Equivalent to `InetAddressPoint.newExactQuery(String, InetAddress)`
    /// (`InetAddressPoint.java:204-206`): a range query whose two bounds are
    /// the same address.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`new_range_query`](Self::new_range_query) raises.
    pub fn new_exact_query(field: &str, value: IpAddr) -> Result<Arc<dyn Query>> {
        Self::new_range_query(field, value, value)
    }

    /// Creates a prefix query matching a CIDR network range.
    ///
    /// Equivalent to `InetAddressPoint.newPrefixQuery(String, InetAddress, int)`
    /// (`InetAddressPoint.java:218-241`): the lower bound zeroes the host
    /// portion of `value`, the upper bound fills it with ones. Java also throws
    /// `IllegalArgumentException("InetAddress must not be null")`; the value
    /// cannot be null here.
    ///
    /// # Errors
    ///
    /// [`LuceneErrorKind::IllegalArgument`] `"illegal prefixLength '…'. Must be
    /// 0-32 for IPv4 ranges, 0-128 for IPv6 ranges"` when `prefix_length` is
    /// negative or longer than the address in bits; propagates whatever
    /// [`new_range_query`](Self::new_range_query) raises.
    pub fn new_prefix_query(
        field: &str,
        value: IpAddr,
        prefix_length: i32,
    ) -> Result<Arc<dyn Query>> {
        let mut lower = get_address(value);
        let mut upper = get_address(value);
        if prefix_length < 0 || prefix_length as usize > 8 * lower.len() {
            return Err(LuceneError::illegal_argument(format!(
                "illegal prefixLength '{prefix_length}'. Must be 0-32 for IPv4 ranges, 0-128 for \
                 IPv6 ranges"
            )));
        }
        // Create the lower value by zeroing out the host portion, the upper
        // value by filling it with all ones.
        for i in (prefix_length as usize)..(8 * lower.len()) {
            let m = 1u8 << (7 - (i & 7));
            lower[i >> 3] &= !m;
            upper[i >> 3] |= m;
        }
        // `InetAddress.getByAddress` of four or sixteen bytes: Java turns its
        // `UnknownHostException` into an `AssertionError`, unreachable.
        let lower = Self::decode(&lower).expect("INVARIANT: an address is 4 or 16 bytes long");
        let upper = Self::decode(&upper).expect("INVARIANT: an address is 4 or 16 bytes long");
        Self::new_range_query(field, lower, upper)
    }

    /// Creates a range query for network addresses, both bounds inclusive.
    ///
    /// A half-open range is expressed with [`MIN_VALUE`](Self::MIN_VALUE) or
    /// [`MAX_VALUE`](Self::MAX_VALUE) on the open side, an exclusive bound with
    /// [`next_up`](Self::next_up) or [`next_down`](Self::next_down).
    ///
    /// Equivalent to `InetAddressPoint.newRangeQuery(String, InetAddress, InetAddress)`
    /// (`InetAddressPoint.java:261-269`), whose anonymous `PointRangeQuery`
    /// subclass (`InetAddressPoint$1`) prints each bound as
    /// `decode(value).getHostAddress()`. `PointRangeQuery.checkArgs` only
    /// rejects null arguments, which cannot arise here.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`PointRangeQuery::new`] raises.
    pub fn new_range_query(
        field: &str,
        lower_value: IpAddr,
        upper_value: IpAddr,
    ) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(PointRangeQuery::new(
            field,
            Self::encode(lower_value),
            Self::encode(upper_value),
            1,
            Arc::new(InetAddressPointFormatter),
        )?))
    }

    /// Creates a query matching any of the given addresses, the points
    /// equivalent of a terms query.
    ///
    /// Equivalent to `InetAddressPoint.newSetQuery(String, InetAddress...)`
    /// (`InetAddressPoint.java:278-305`): the addresses are encoded and the
    /// encodings sorted unsigned, because `InetAddress` is not comparable and
    /// the IPv4 mapping defines the indexed order. Each value prints as
    /// `decode(value).getHostAddress()` (the anonymous `InetAddressPoint$3`).
    ///
    /// # Errors
    ///
    /// Propagates whatever [`PointInSetQuery::new`] raises.
    pub fn new_set_query(field: &str, values: &[IpAddr]) -> Result<Arc<dyn Query>> {
        let mut sorted_values: Vec<Vec<u8>> = values.iter().map(|v| Self::encode(*v)).collect();
        // `Arrays.compareUnsigned` over sixteen bytes is the lexicographic
        // order of `Vec<u8>`.
        sorted_values.sort();
        let mut stream = PointValueStream::new(sorted_values);
        Ok(Arc::new(PointInSetQuery::new(
            field,
            1,
            Self::BYTES as i32,
            &mut stream,
            Arc::new(InetAddressPointFormatter),
        )?))
    }
}

/// The address as Java holds it: an IPv4-mapped [`IpAddr::V6`] is the
/// `Inet4Address` it maps, which is what `InetAddress.getByName` and
/// `InetAddress.getByAddress` return for it.
fn canonical(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => address,
        },
        v4 => v4,
    }
}

/// `InetAddress.getAddress()`: four bytes for an IPv4 address, sixteen for
/// an IPv6 address, read through [`canonical`].
fn get_address(address: IpAddr) -> Vec<u8> {
    match canonical(address) {
        IpAddr::V4(v4) => v4.octets().to_vec(),
        IpAddr::V6(v6) => v6.octets().to_vec(),
    }
}

/// `InetAddress.equals(Object)`: both `Inet4Address` or both `Inet6Address`,
/// with the same address bytes.
fn java_equals(a: IpAddr, b: IpAddr) -> bool {
    canonical(a) == canonical(b)
}

/// `InetAddress.getHostAddress()`: `Inet4Address` prints four decimal
/// octets; `Inet6Address` prints eight lowercase hexadecimal groups without
/// leading zeros and *without* the `::` compression Rust's [`Ipv6Addr`]
/// `Display` applies.
fn get_host_address(address: IpAddr) -> String {
    match canonical(address) {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            format!("{}.{}.{}.{}", o[0], o[1], o[2], o[3])
        }
        IpAddr::V6(v6) => v6
            .segments()
            .iter()
            .map(|s| format!("{s:x}"))
            .collect::<Vec<_>>()
            .join(":"),
    }
}

/// Encodes an address given as its four or sixteen raw bytes, as
/// `InetAddressPoint.encode(InetAddress)` encodes the `InetAddress` those
/// bytes build.
///
/// The bridge for [`InetAddressRange`](crate::document::geo_fields::InetAddressRange),
/// whose port still takes raw address bytes where Java takes an
/// `InetAddress`; it keeps that type's behaviour, including its error for a
/// length that is neither four nor sixteen.
pub(crate) fn encode_address_bytes(address: &[u8]) -> Result<Vec<u8>> {
    match address.len() {
        4 | 16 => Ok(InetAddressPoint::encode(InetAddressPoint::decode(address)?)),
        other => Err(LuceneError::illegal_argument(format!(
            "invalid address length {other}; must be 4 or 16 bytes"
        ))),
    }
}

/// Renders an address given as its four or sixteen bytes the way Java
/// prints `InetAddressPoint.decode(value).getHostAddress()`.
///
/// For a value of any other length Java's `decode` throws; a `toString`
/// cannot fail, so the bytes are printed in hexadecimal instead.
pub(crate) fn inet_address_host_address(value: &[u8]) -> String {
    match InetAddressPoint::decode(value) {
        Ok(address) => get_host_address(address),
        Err(_) => value.iter().map(|b| format!("{b:02x}")).collect(),
    }
}

/// Renders an encoded address as a host address in a query's string form.
///
/// Equivalent to the anonymous `PointRangeQuery` subclass `InetAddressPoint$1`
/// (`InetAddressPoint.java:263-268`) and the anonymous `PointInSetQuery`
/// subclass `InetAddressPoint$3` (`InetAddressPoint.java:300-304`), whose only
/// override prints `decode(value).getHostAddress()`.
#[derive(Debug)]
struct InetAddressPointFormatter;

impl DimensionFormatter for InetAddressPointFormatter {
    /// For ranges, the range itself is already bracketed.
    fn format(&self, _dimension: i32, value: &[u8]) -> String {
        inet_address_host_address(value)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// The anonymous subclass `org.apache.lucene.document.InetAddressPoint$1`.
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.InetAddressPoint$1"
    }
}

impl PointFormatter for InetAddressPointFormatter {
    fn format(&self, value: &[u8]) -> String {
        debug_assert!(value.len() == InetAddressPoint::BYTES);
        inet_address_host_address(value)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// The anonymous subclass `org.apache.lucene.document.InetAddressPoint$3`.
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.InetAddressPoint$3"
    }
}

impl std::fmt::Display for InetAddressPoint {
    /// Equivalent to `InetAddressPoint.toString()`
    /// (`InetAddressPoint.java:141-165`): the class name, then the field name
    /// and the decoded address between angle brackets, an IPv6 address
    /// bracketed so that it cannot be confused with the historic
    /// `field:value` form.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "InetAddressPoint <{}:", self.name)?;
        if let Some(bytes) = self.packed_value() {
            match Self::decode(&bytes) {
                Ok(address @ IpAddr::V6(_)) => write!(f, "[{}]", get_host_address(address))?,
                Ok(address) => f.write_str(&get_host_address(address))?,
                Err(_) => f.write_str(&inet_address_host_address(&bytes))?,
            }
        }
        f.write_str(">")
    }
}

/// `InetAddressPoint` inherits `Field`'s setters and overrides
/// `setBytesValue` to throw (`InetAddressPoint.java:127-130`).
impl FieldValueSetters for InetAddressPoint {
    fn fields_data_class(&self) -> &'static str {
        fields_data_simple_name(&self.fields_data.borrow())
    }

    fn set_bytes_value(&self, _value: BytesRef) -> Result<()> {
        Err(LuceneError::illegal_argument(
            "cannot change value type from InetAddress to BytesRef".to_string(),
        ))
    }
}

impl IndexableField for InetAddressPoint {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &*self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Ok(Box::new(crate::analysis::BinaryTokenStream::new(value)?))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &*self.fields_data.borrow() {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

#[cfg(test)]
mod tests {
    //! The cases of Lucene's `TestInetAddressPoint`
    //! (`lucene/core/src/test/org/apache/lucene/document/TestInetAddressPoint.java`),
    //! plus the field and error contracts of `InetAddressPoint`.

    use super::*;
    use crate::codecs::{register_codec, Lucene104Codec};
    use crate::document::Document;
    use crate::index::index_writer::IndexWriter;
    use crate::index::index_writer_config::IndexWriterConfig;
    use crate::index::{directory_reader, IndexReader};
    use crate::search::IndexSearcher;
    use crate::store::{ByteBuffersDirectory, Directory};

    /// `InetAddress.getByName(literal)` for an address literal.
    fn ip(literal: &str) -> IpAddr {
        literal.parse().unwrap()
    }

    fn message(e: &LuceneError) -> String {
        match e.kind() {
            LuceneErrorKind::IllegalArgument { message, .. } => message.clone(),
            LuceneErrorKind::Arithmetic(message) => message.clone(),
            other => panic!("unexpected error kind {other:?}"),
        }
    }

    /// Indexes one document holding `address` and returns a searcher over it.
    fn index_one(address: IpAddr) -> IndexSearcher {
        let _ = register_codec("Lucene104", Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        let mut document = Document::new();
        document.add(Box::new(InetAddressPoint::new("field", address)));
        writer.add_document(&document).unwrap();
        writer.commit().unwrap();
        writer.close().unwrap();
        let reader: Arc<dyn IndexReader> = directory_reader::open(dir).unwrap();
        IndexSearcher::new(reader).unwrap()
    }

    fn count(searcher: &IndexSearcher, query: Arc<dyn Query>) -> i32 {
        searcher.count(query).unwrap()
    }

    /// `TestInetAddressPoint.testBasics`.
    #[test]
    fn test_basics() {
        let address = ip("1.2.3.4");
        let s = index_one(address);
        let q = |r: Result<Arc<dyn Query>>| r.unwrap();
        assert_eq!(
            1,
            count(&s, q(InetAddressPoint::new_exact_query("field", address)))
        );
        assert_eq!(
            1,
            count(
                &s,
                q(InetAddressPoint::new_prefix_query("field", address, 24))
            )
        );
        assert_eq!(
            1,
            count(
                &s,
                q(InetAddressPoint::new_range_query(
                    "field",
                    ip("1.2.3.3"),
                    ip("1.2.3.5")
                ))
            )
        );
        assert_eq!(
            1,
            count(
                &s,
                q(InetAddressPoint::new_set_query("field", &[ip("1.2.3.4")]))
            )
        );
        assert_eq!(
            1,
            count(
                &s,
                q(InetAddressPoint::new_set_query(
                    "field",
                    &[ip("1.2.3.4"), ip("1.2.3.5")]
                ))
            )
        );
        assert_eq!(
            0,
            count(
                &s,
                q(InetAddressPoint::new_set_query("field", &[ip("1.2.3.3")]))
            )
        );
        assert_eq!(
            0,
            count(&s, q(InetAddressPoint::new_set_query("field", &[])))
        );
    }

    /// `TestInetAddressPoint.testBasicsV6`.
    #[test]
    fn test_basics_v6() {
        let address = ip("fec0::f66d");
        let s = index_one(address);
        assert_eq!(
            1,
            count(
                &s,
                InetAddressPoint::new_exact_query("field", address).unwrap()
            )
        );
        assert_eq!(
            1,
            count(
                &s,
                InetAddressPoint::new_prefix_query("field", address, 64).unwrap()
            )
        );
        assert_eq!(
            1,
            count(
                &s,
                InetAddressPoint::new_range_query("field", ip("fec0::f66c"), ip("fec0::f66e"))
                    .unwrap()
            )
        );
    }

    /// `TestInetAddressPoint.testToString`.
    #[test]
    fn test_to_string() {
        assert_eq!(
            "InetAddressPoint <field:1.2.3.4>",
            InetAddressPoint::new("field", ip("1.2.3.4")).to_string()
        );
        assert_eq!(
            "InetAddressPoint <field:1.2.3.4>",
            InetAddressPoint::new("field", ip("::FFFF:1.2.3.4")).to_string()
        );
        assert_eq!(
            "InetAddressPoint <field:[fdc8:57ed:f042:ad1:f66d:4ff:fe90:ce0c]>",
            InetAddressPoint::new("field", ip("fdc8:57ed:f042:0ad1:f66d:4ff:fe90:ce0c"))
                .to_string()
        );
        assert_eq!(
            "field:[1.2.3.4 TO 1.2.3.4]",
            InetAddressPoint::new_exact_query("field", ip("1.2.3.4"))
                .unwrap()
                .to_query_string("")
        );
        assert_eq!(
            "field:[0:0:0:0:0:0:0:1 TO 0:0:0:0:0:0:0:1]",
            InetAddressPoint::new_exact_query("field", ip("::1"))
                .unwrap()
                .to_query_string("")
        );
        assert_eq!(
            "field:[1.2.3.0 TO 1.2.3.255]",
            InetAddressPoint::new_prefix_query("field", ip("1.2.3.4"), 24)
                .unwrap()
                .to_query_string("")
        );
        assert_eq!(
            "field:[fdc8:57ed:f042:ad1:0:0:0:0 TO fdc8:57ed:f042:ad1:ffff:ffff:ffff:ffff]",
            InetAddressPoint::new_prefix_query(
                "field",
                ip("fdc8:57ed:f042:0ad1:f66d:4ff:fe90:ce0c"),
                64
            )
            .unwrap()
            .to_query_string("")
        );
        assert_eq!(
            "field:{fdc8:57ed:f042:ad1:f66d:4ff:fe90:ce0c}",
            InetAddressPoint::new_set_query(
                "field",
                &[ip("fdc8:57ed:f042:0ad1:f66d:4ff:fe90:ce0c")]
            )
            .unwrap()
            .to_query_string("")
        );
    }

    fn assert_query_eq(a: &Arc<dyn Query>, b: &Arc<dyn Query>) {
        assert!(a.query_eq(b.as_ref()));
        assert_eq!(a.query_hash(), b.query_hash());
    }

    /// `TestInetAddressPoint.testQueryEquals`.
    #[test]
    fn test_query_equals() {
        let range =
            |f: &str, a: &str, b: &str| InetAddressPoint::new_range_query(f, ip(a), ip(b)).unwrap();
        let q1 = range("a", "1.2.3.3", "1.2.3.5");
        assert_query_eq(&q1, &range("a", "1.2.3.3", "1.2.3.5"));
        assert!(!q1.query_eq(range("a", "1.2.3.3", "1.2.3.7").as_ref()));
        assert!(!q1.query_eq(range("b", "1.2.3.3", "1.2.3.5").as_ref()));

        let prefix = |a: &str, p: i32| InetAddressPoint::new_prefix_query("a", ip(a), p).unwrap();
        let q1 = prefix("1.2.3.3", 16);
        assert_query_eq(&q1, &prefix("1.2.3.3", 16));
        assert!(!q1.query_eq(prefix("1.1.3.5", 16).as_ref()));
        assert!(!q1.query_eq(prefix("1.2.3.5", 24).as_ref()));

        let exact = |a: &str| InetAddressPoint::new_exact_query("a", ip(a)).unwrap();
        let q1 = exact("1.2.3.3");
        assert_query_eq(&q1, &exact("1.2.3.3"));
        assert!(!q1.query_eq(exact("1.2.3.5").as_ref()));

        let set = |a: &str, b: &str| InetAddressPoint::new_set_query("a", &[ip(a), ip(b)]).unwrap();
        let q1 = set("1.2.3.3", "1.2.3.5");
        assert_query_eq(&q1, &set("1.2.3.3", "1.2.3.5"));
        assert!(!q1.query_eq(set("1.2.3.3", "1.2.3.7").as_ref()));
    }

    /// `TestInetAddressPoint.testPrefixQuery`.
    #[test]
    fn test_prefix_query() {
        let cases = [
            ("1.2.3.0", "1.2.3.255", "1.2.3.127", 24),
            ("1.2.3.128", "1.2.3.255", "1.2.3.213", 25),
            ("2001::a000:0", "2001::afff:ffff", "2001::a6bd:fc80", 100),
        ];
        for (lower, upper, value, prefix) in cases {
            assert_query_eq(
                &InetAddressPoint::new_range_query("a", ip(lower), ip(upper)).unwrap(),
                &InetAddressPoint::new_prefix_query("a", ip(value), prefix).unwrap(),
            );
        }
    }

    /// `TestInetAddressPoint.testNextUp`.
    #[test]
    fn test_next_up() {
        let cases = [
            ("::1", "::"),
            ("::1:0", "::ffff"),
            ("1.2.4.0", "1.2.3.255"),
            ("0.0.0.0", "::fffe:ffff:ffff"),
            ("::1:0:0:0", "255.255.255.255"),
        ];
        for (expected, input) in cases {
            assert_eq!(ip(expected), InetAddressPoint::next_up(ip(input)).unwrap());
        }
        let e =
            InetAddressPoint::next_up(ip("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff")).unwrap_err();
        assert!(matches!(e.kind(), LuceneErrorKind::Arithmetic(_)));
        assert_eq!(
            "Overflow: there is no greater InetAddress than ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
            message(&e)
        );
    }

    /// `TestInetAddressPoint.testNextDown`.
    #[test]
    fn test_next_down() {
        let cases = [
            (
                "ffff:ffff:ffff:ffff:ffff:ffff:ffff:fffe",
                "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
            ),
            ("::ffff", "::1:0"),
            ("1.2.3.255", "1.2.4.0"),
            ("::fffe:ffff:ffff", "0.0.0.0"),
            ("255.255.255.255", "::1:0:0:0"),
        ];
        for (expected, input) in cases {
            assert_eq!(
                ip(expected),
                InetAddressPoint::next_down(ip(input)).unwrap()
            );
        }
        let e = InetAddressPoint::next_down(ip("::")).unwrap_err();
        assert!(matches!(e.kind(), LuceneErrorKind::Arithmetic(_)));
        assert_eq!(
            "Underflow: there is no smaller InetAddress than 0:0:0:0:0:0:0:0",
            message(&e)
        );
    }

    /// `encode` maps IPv4 into IPv6 space; `decode` hands an IPv4-mapped
    /// encoding back as the IPv4 address, as `InetAddress.getByAddress` does.
    #[test]
    fn encode_and_decode_follow_get_by_address() {
        let mapped = InetAddressPoint::encode(ip("10.0.0.1"));
        assert_eq!(
            mapped,
            vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 10, 0, 0, 1]
        );
        assert_eq!(mapped, InetAddressPoint::encode(ip("::ffff:10.0.0.1")));
        assert_eq!(ip("10.0.0.1"), InetAddressPoint::decode(&mapped).unwrap());
        assert_eq!(
            ip("10.0.0.1"),
            InetAddressPoint::decode(&[10, 0, 0, 1]).unwrap()
        );
        // An IPv4-compatible address stays IPv6.
        assert_eq!(
            ip("::1.2.3.4"),
            InetAddressPoint::decode(&InetAddressPoint::encode(ip("::1.2.3.4"))).unwrap()
        );
        assert!(matches!(
            InetAddressPoint::decode(&InetAddressPoint::encode(ip("::1.2.3.4"))).unwrap(),
            IpAddr::V6(_)
        ));
        for len in [0usize, 3, 5, 15, 17] {
            let e = InetAddressPoint::decode(&vec![0u8; len]).unwrap_err();
            assert!(e.is_illegal_argument_exception());
            assert_eq!("encoded bytes are of incorrect length", message(&e));
        }
        assert_eq!(
            InetAddressPoint::MIN_VALUE,
            InetAddressPoint::decode(&[0u8; 16]).unwrap()
        );
        assert_eq!(
            InetAddressPoint::MAX_VALUE,
            InetAddressPoint::decode(&[0xFFu8; 16]).unwrap()
        );
    }

    /// The prefix length is checked against `8 * getAddress().length`, which
    /// is 32 for an IPv4-mapped `IpAddr::V6` too, as Java holds it as an
    /// `Inet4Address`.
    #[test]
    fn new_prefix_query_rejects_an_illegal_prefix_length() {
        let cases = [
            ("1.2.3.4", -1),
            ("1.2.3.4", 33),
            ("::ffff:1.2.3.4", 33),
            ("::1", 129),
        ];
        for (address, prefix) in cases {
            let e = InetAddressPoint::new_prefix_query("f", ip(address), prefix).unwrap_err();
            assert!(e.is_illegal_argument_exception());
            assert_eq!(
                format!(
                    "illegal prefixLength '{prefix}'. Must be 0-32 for IPv4 ranges, 0-128 for \
                     IPv6 ranges"
                ),
                message(&e)
            );
        }
        assert!(InetAddressPoint::new_prefix_query("f", ip("::1"), 33).is_ok());
    }

    /// The field: one dimension of sixteen bytes, the shared static type,
    /// `setInetAddressValue`, and `setBytesValue`'s refusal.
    #[test]
    fn field_value_and_type() {
        let field = InetAddressPoint::new("f", ip("1.2.3.4"));
        let other = InetAddressPoint::new("g", ip("::1"));
        assert!(Arc::ptr_eq(&field.field_type, &other.field_type));
        assert_eq!(1, field.field_type().point_dimension_count());
        assert_eq!(16, field.field_type().point_num_bytes());
        assert_eq!(
            InetAddressPoint::encode(ip("1.2.3.4")),
            field.binary_value().unwrap().slice()
        );
        field.set_inet_address_value(ip("::1"));
        assert_eq!("InetAddressPoint <f:[0:0:0:0:0:0:0:1]>", field.to_string());
        let e = FieldValueSetters::set_bytes_value(&field, BytesRef::new(vec![1])).unwrap_err();
        assert_eq!(
            "cannot change value type from InetAddress to BytesRef",
            message(&e)
        );
        let e = FieldValueSetters::set_string_value(&field, "x".to_string()).unwrap_err();
        assert_eq!(
            "cannot change value type from BytesRef to String",
            message(&e)
        );
    }
}
