// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.ICUBinary`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use rucene::error::{LuceneError, Result};

use crate::jdk::{index_out_of_bounds, ByteBuffer, ByteOrder, ClassLoader, DataOutputStream, InputStream};
use crate::r#impl::class_loader_util::ClassLoaderUtil;
use crate::r#impl::icu_config::ICUConfig;
use crate::r#impl::icu_data::ICUData;
use crate::util::icu_unchecked_io_exception::ICUUncheckedIOException;
use crate::util::version_info::VersionInfo;

/// `java.io.IOException(message)`.
fn io_exception(message: impl Into<String>) -> LuceneError {
    LuceneError::from(std::io::Error::other(message.into()))
}

/// The reader of ICU `.dat` package files: a table of contents of item
/// names and offsets after a `CmnD` data header.
///
/// Equivalent to `ICUBinary.DatPackageReader` (private).
struct DatPackageReader;

impl DatPackageReader {
    /// `DATA_FORMAT`: `"CmnD"`.
    const DATA_FORMAT: i32 = 0x436d6e44;

    /// `IS_ACCEPTABLE`: format version 1.
    fn is_acceptable(version: &[u8; 4]) -> bool {
        version[0] == 1
    }

    /// `validate(ByteBuffer bytes)`: whether `bytes` is an ICU package of
    /// this ICU version; reads the header, so the position moves past it.
    fn validate(bytes: &mut ByteBuffer) -> bool {
        if ICUBinary::read_header(bytes, Self::DATA_FORMAT, Some(&Self::is_acceptable)).is_err() {
            return false;
        }
        let count = bytes.get_int_at(bytes.position());
        if count <= 0 {
            return false;
        }
        // For each item, there is one ToC entry (8 bytes) and a name string
        // and a data item of at least 16 bytes.
        if bytes.position() as i64 + 4 + count as i64 * (8 + 16) > bytes.capacity() as i64 {
            return false;
        }
        if !Self::starts_with_package_name(bytes, Self::get_name_offset(bytes, 0))
            || !Self::starts_with_package_name(bytes, Self::get_name_offset(bytes, count - 1))
        {
            return false;
        }
        true
    }

    /// `startsWithPackageName(ByteBuffer bytes, int start)` (private): the
    /// name at `start` begins with `icudt77b/` or `icudt77l/`.
    fn starts_with_package_name(bytes: &ByteBuffer, start: i32) -> bool {
        let package_name = ICUData::package_name();
        let pn = package_name.as_bytes();
        // Compare all but the trailing 'b' or 'l' which depends on the platform.
        let mut length = pn.len() as i32 - 1;
        for i in 0..length {
            if bytes.get_at(start + i) as i32 != pn[i as usize] as i32 {
                return false;
            }
        }
        // Check for 'b' or 'l' followed by '/'.
        let c = bytes.get_at(start + length);
        length += 1;
        !((c != b'b' as i8 && c != b'l' as i8) || bytes.get_at(start + length) != b'/' as i8)
    }

    /// `getData(ByteBuffer bytes, CharSequence key)`: the item `key` as a
    /// slice of the package with its byte order, `None` when absent.
    fn get_data(bytes: &ByteBuffer, key: &str) -> Result<Option<ByteBuffer>> {
        let index = Self::binary_search(bytes, key);
        if index >= 0 {
            let mut data = bytes.duplicate();
            data.set_position(Self::get_data_offset(bytes, index))?;
            data.set_limit(Self::get_data_offset(bytes, index + 1))?;
            // `duplicate()` gives the big-endian order of a new buffer, which
            // the slice keeps, as in Java; the item's own header sets it.
            Ok(Some(ICUBinary::slice_with_order(&data)))
        } else {
            Ok(None)
        }
    }

    /// `addBaseNamesInFolder(ByteBuffer, String folder, String suffix,
    /// Set<String> names)`.
    fn add_base_names_in_folder(
        bytes: &ByteBuffer,
        folder: &str,
        suffix: &str,
        names: &mut HashSet<String>,
    ) {
        // Find the first data item name that starts with the folder name.
        let mut index = Self::binary_search(bytes, folder);
        if index < 0 {
            index = !index;
        }
        let base = bytes.position();
        let count = bytes.get_int_at(base);
        let mut sb: Vec<u16> = Vec::new();
        while index < count && Self::add_base_name(bytes, index, folder, suffix, &mut sb, names) {
            index += 1;
        }
    }

    /// `binarySearch(ByteBuffer bytes, CharSequence key)` (private): the
    /// index of `key`, or `~insertionPoint`.
    fn binary_search(bytes: &ByteBuffer, key: &str) -> i32 {
        let base = bytes.position();
        let count = bytes.get_int_at(base);
        // Do a binary search for the key.
        let mut start = 0i32;
        let mut limit = count;
        let key: Vec<u16> = key.encode_utf16().collect();
        let skip = ICUData::package_name().len() as i32 + 1;
        while start < limit {
            let mid = ((start as u32 + limit as u32) >> 1) as i32;
            let mut name_offset = Self::get_name_offset(bytes, mid);
            // Skip "icudt54b/".
            name_offset += skip;
            let result = ICUBinary::compare_keys_buffer(&key, bytes, name_offset);
            if result < 0 {
                limit = mid;
            } else if result > 0 {
                start = mid + 1;
            } else {
                // We found it!
                return mid;
            }
        }
        !start // Not found or table is empty.
    }

    /// `getNameOffset(ByteBuffer bytes, int index)` (private).
    fn get_name_offset(bytes: &ByteBuffer, index: i32) -> i32 {
        let base = bytes.position();
        debug_assert!(0 <= index && index < bytes.get_int_at(base)); // count
        // The count integer (4 bytes)
        // is followed by count (nameOffset, dataOffset) integer pairs (8 bytes per pair).
        base.wrapping_add(bytes.get_int_at(base + 4 + index * 8))
    }

    /// `getDataOffset(ByteBuffer bytes, int index)` (private).
    fn get_data_offset(bytes: &ByteBuffer, index: i32) -> i32 {
        let base = bytes.position();
        let count = bytes.get_int_at(base);
        if index == count {
            // Return the limit of the last data item.
            return bytes.capacity();
        }
        debug_assert!(0 <= index && index < count);
        // The count integer (4 bytes)
        // is followed by count (nameOffset, dataOffset) integer pairs (8 bytes per pair).
        // The dataOffset follows the nameOffset (skip another 4 bytes).
        base.wrapping_add(bytes.get_int_at(base + 4 + 4 + index * 8))
    }

    /// `addBaseName(ByteBuffer, int index, String folder, String suffix,
    /// StringBuilder sb, Set<String> names)`: `false` once the names leave
    /// `folder`.
    fn add_base_name(
        bytes: &ByteBuffer,
        index: i32,
        folder: &str,
        suffix: &str,
        sb: &mut Vec<u16>,
        names: &mut HashSet<String>,
    ) -> bool {
        let mut offset = Self::get_name_offset(bytes, index);
        // Skip "icudt54b/".
        offset += ICUData::package_name().len() as i32 + 1;
        if !folder.is_empty() {
            // Test name.startsWith(folder + '/').
            for &fc in folder.as_bytes() {
                if bytes.get_at(offset) as i32 != fc as i32 {
                    return false;
                }
                offset += 1;
            }
            let b = bytes.get_at(offset);
            offset += 1;
            if b != b'/' as i8 {
                return false;
            }
        }
        // Collect the NUL-terminated name and test for a subfolder, then test for the suffix.
        sb.clear();
        loop {
            let b = bytes.get_at(offset);
            offset += 1;
            if b == 0 {
                break;
            }
            let c = b as u16; // (char) b
            if c == b'/' as u16 {
                return true; // NUL-terminated name.
            }
            sb.push(c);
        }
        let suffix_units: Vec<u16> = suffix.encode_utf16().collect();
        let name_limit = sb.len() as i32 - suffix_units.len() as i32;
        if crate::jdk::last_index_of(sb, &suffix_units, name_limit) >= 0 {
            names.insert(String::from_utf16_lossy(&sb[..name_limit as usize]));
        }
        true
    }
}

/// A source of ICU data items: one file of a data folder, or a `.dat`
/// package.
///
/// Equivalent to `ICUBinary.DataFile` (private abstract class).
#[allow(dead_code)] // `itemPath` and `toString()` serve debugging in Java too.
trait DataFile: Send + Sync {
    /// `itemPath`.
    fn item_path(&self) -> &str;

    /// `getData(String requestedPath)`.
    fn get_data(&self, requested_path: &str) -> Result<Option<ByteBuffer>>;

    /// `addBaseNamesInFolder(String folder, String suffix, Set<String> names)`.
    fn add_base_names_in_folder(&self, folder: &str, suffix: &str, names: &mut HashSet<String>);

    /// `toString()`.
    fn to_java_string(&self) -> String {
        self.item_path().to_string()
    }
}

/// Equivalent to `ICUBinary.SingleDataFile` (private).
struct SingleDataFile {
    item_path: String,
    path: PathBuf,
}

impl DataFile for SingleDataFile {
    fn item_path(&self) -> &str {
        &self.item_path
    }

    fn to_java_string(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }

    fn get_data(&self, requested_path: &str) -> Result<Option<ByteBuffer>> {
        if requested_path == self.item_path {
            Ok(ICUBinary::map_file(&self.path))
        } else {
            Ok(None)
        }
    }

    fn add_base_names_in_folder(&self, folder: &str, suffix: &str, names: &mut HashSet<String>) {
        let item: Vec<u16> = self.item_path.encode_utf16().collect();
        let f: Vec<u16> = folder.encode_utf16().collect();
        let s: Vec<u16> = suffix.encode_utf16().collect();
        if item.len() > f.len() + s.len()
            && item.starts_with(&f)
            && item.ends_with(&s)
            && item[f.len()] == b'/' as u16
            && crate::jdk::index_of_unit(&item, b'/' as u16, f.len() as i32 + 1) < 0
        {
            names.insert(String::from_utf16_lossy(&item[f.len() + 1..item.len() - s.len()]));
        }
    }
}

/// Equivalent to `ICUBinary.PackageDataFile` (private).
#[allow(dead_code)]
struct PackageDataFile {
    item_path: String,
    /// The ByteBuffer pointing to the beginning of the TOC.
    pkg_bytes: ByteBuffer,
}

impl DataFile for PackageDataFile {
    fn item_path(&self) -> &str {
        &self.item_path
    }

    fn get_data(&self, requested_path: &str) -> Result<Option<ByteBuffer>> {
        DatPackageReader::get_data(&self.pkg_bytes, requested_path)
    }

    fn add_base_names_in_folder(&self, folder: &str, suffix: &str, names: &mut HashSet<String>) {
        DatPackageReader::add_base_names_in_folder(&self.pkg_bytes, folder, suffix, names);
    }
}

/// `icuDataFiles`: the data files and packages found under the folders of
/// the `com.ibm.icu.impl.ICUBinary.dataPath` setting, which the static
/// initializer reads (the shipped `ICUConfig.properties` sets it empty).
static ICU_DATA_FILES: LazyLock<Vec<Box<dyn DataFile>>> = LazyLock::new(|| {
    let mut files: Vec<Box<dyn DataFile>> = Vec::new();
    // Normally com.ibm.icu.impl.ICUBinary.dataPath.
    if let Some(data_path) = ICUConfig::get("com.ibm.icu.impl.ICUBinary.dataPath") {
        ICUBinary::add_data_files_from_path(&data_path, &mut files);
    }
    files
});

/// The `ICUBinary.Authenticate` callback: whether a data file's format
/// version is acceptable.
///
/// Equivalent to the interface `ICUBinary.Authenticate`.
pub type Authenticate<'a> = &'a dyn Fn(&[u8; 4]) -> bool;

/// Reading of ICU binary data: the data files, their common header, and
/// typed arrays from a [`ByteBuffer`].
///
/// Equivalent to `com.ibm.icu.impl.ICUBinary`.
pub struct ICUBinary;

impl ICUBinary {
    /// `MAGIC1`.
    const MAGIC1: i8 = 0xda_u8 as i8;
    /// `MAGIC2`.
    const MAGIC2: i8 = 0x27;
    /// `CHAR_SET_`.
    const CHAR_SET: i8 = 0;
    /// `CHAR_SIZE_`.
    const CHAR_SIZE: i8 = 2;
    /// `MAGIC_NUMBER_AUTHENTICATION_FAILED_`.
    const MAGIC_NUMBER_AUTHENTICATION_FAILED: &'static str =
        "ICU data file error: Not an ICU data file";
    /// `HEADER_AUTHENTICATION_FAILED_`.
    const HEADER_AUTHENTICATION_FAILED: &'static str =
        "ICU data file error: Header authentication failed, please check if you have a valid ICU data file";

    /// `addDataFilesFromPath(String dataPath, List<DataFile> files)`
    /// (private): the folders of the path list, separated by
    /// `File.pathSeparatorChar`.
    fn add_data_files_from_path(data_path: &str, files: &mut Vec<Box<dyn DataFile>>) {
        // Split the path and find files in each location.
        // This splitting code avoids the regex pattern compilation in String.split()
        // and its array allocation.
        // (There is no simple by-character split()
        // and the StringTokenizer "is discouraged in new code".)
        let sep = if cfg!(windows) { ';' } else { ':' };
        let file_separator = std::path::MAIN_SEPARATOR_STR;
        let units: Vec<u16> = data_path.encode_utf16().collect();
        let mut path_start = 0i32;
        while (path_start as usize) < units.len() {
            let sep_index = crate::jdk::index_of_unit(&units, sep as u16, path_start);
            let path_limit = if sep_index >= 0 {
                sep_index
            } else {
                units.len() as i32
            };
            let raw = String::from_utf16_lossy(&units[path_start as usize..path_limit as usize]);
            let mut path = java_trim(&raw).to_string();
            if path.ends_with(file_separator) {
                path.truncate(path.len() - file_separator.len());
            }
            if !path.is_empty() {
                Self::add_data_files_from_folder(Path::new(&path), &mut String::new(), files);
            }
            if sep_index < 0 {
                break;
            }
            path_start = sep_index + 1;
        }
    }

    /// `addDataFilesFromFolder(File folder, StringBuilder itemPath,
    /// List<DataFile> dataFiles)` (private), in the order the folder lists
    /// its entries (`File.listFiles`, the directory's own order).
    fn add_data_files_from_folder(
        folder: &Path,
        item_path: &mut String,
        data_files: &mut Vec<Box<dyn DataFile>>,
    ) {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };
        let files: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        if files.is_empty() {
            return;
        }
        let mut folder_path_length = item_path.len();
        if folder_path_length > 0 {
            // The item path must use the ICU file separator character,
            // not the platform-dependent File.separatorChar,
            // so that the enumerated item paths match the paths requested by ICU code.
            item_path.push('/');
            folder_path_length += 1;
        }
        for file in files {
            let file_name = file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if file_name.ends_with(".txt") {
                continue;
            }
            item_path.push_str(&file_name);
            if file.is_dir() {
                // TODO: Within a folder, put all single files before all .dat packages?
                Self::add_data_files_from_folder(&file, item_path, data_files);
            } else if file_name.ends_with(".dat") {
                if let Some(mut pkg_bytes) = Self::map_file(&file) {
                    if DatPackageReader::validate(&mut pkg_bytes) {
                        data_files.push(Box::new(PackageDataFile {
                            item_path: item_path.clone(),
                            pkg_bytes,
                        }));
                    }
                }
            } else {
                data_files.push(Box::new(SingleDataFile {
                    item_path: item_path.clone(),
                    path: file.clone(),
                }));
            }
            item_path.truncate(folder_path_length);
        }
    }

    /// `compareKeys(CharSequence key, ByteBuffer bytes, int offset)`
    /// (package-private): compares the key with the NUL-terminated
    /// invariant-character name at `offset`, the bytes read as signed
    /// values.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` when the name runs past the limit.
    pub fn compare_keys_buffer(key: &[u16], bytes: &ByteBuffer, mut offset: i32) -> i32 {
        let mut i = 0usize;
        loop {
            let c2 = bytes.get_at(offset) as i32;
            if c2 == 0 {
                if i == key.len() {
                    return 0;
                } else {
                    return 1; // key > table key because key is longer.
                }
            } else if i == key.len() {
                return -1; // key < table key because key is shorter.
            }
            let diff = key[i] as i32 - c2;
            if diff != 0 {
                return diff;
            }
            i += 1;
            offset += 1;
        }
    }

    /// `compareKeys(CharSequence key, byte[] bytes, int offset)`
    /// (package-private).
    ///
    /// # Panics
    ///
    /// Where Java throws `ArrayIndexOutOfBoundsException`: the name runs
    /// past the array.
    pub fn compare_keys_bytes(key: &[u16], bytes: &[i8], mut offset: i32) -> i32 {
        let mut i = 0usize;
        loop {
            let c2 = bytes[offset as usize] as i32;
            if c2 == 0 {
                if i == key.len() {
                    return 0;
                } else {
                    return 1; // key > table key because key is longer.
                }
            } else if i == key.len() {
                return -1; // key < table key because key is shorter.
            }
            let diff = key[i] as i32 - c2;
            if diff != 0 {
                return diff;
            }
            i += 1;
            offset += 1;
        }
    }

    /// `getData(String itemPath)`: the data item at `itemPath` (relative to
    /// the ICU data root), `None` when it is missing.
    ///
    /// # Errors
    ///
    /// `ICUUncheckedIOException` when reading the resource fails.
    pub fn get_data(item_path: &str) -> Result<Option<ByteBuffer>> {
        Self::get_data_impl(None, None, item_path, false)
    }

    /// `getData(ClassLoader loader, String resourceName, String itemPath)`:
    /// `loader` and `resourceName` default to the loader of `ICUData` and
    /// `ICU_BASE_NAME + '/' + itemPath`.
    ///
    /// # Errors
    ///
    /// `ICUUncheckedIOException` when reading the resource fails.
    pub fn get_data_with(
        loader: Option<&ClassLoader>,
        resource_name: Option<&str>,
        item_path: &str,
    ) -> Result<Option<ByteBuffer>> {
        Self::get_data_impl(loader, resource_name, item_path, false)
    }

    /// `getRequiredData(String itemPath)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the item is missing;
    /// `ICUUncheckedIOException` when reading it fails.
    pub fn get_required_data(item_path: &str) -> Result<ByteBuffer> {
        Self::get_data_impl(None, None, item_path, true)
            .map(|d| d.expect("a required lookup returns data or throws"))
    }

    /// `getData(ClassLoader, String, String, boolean required)` (private):
    /// the data files of the data path first, then the class-path resource.
    fn get_data_impl(
        loader: Option<&ClassLoader>,
        resource_name: Option<&str>,
        item_path: &str,
        required: bool,
    ) -> Result<Option<ByteBuffer>> {
        if let Some(bytes) = Self::get_data_from_file(item_path)? {
            return Ok(Some(bytes));
        }
        let loader = match loader {
            Some(l) => *l,
            None => ClassLoaderUtil::get_class_loader_of(&ICUData::CLASS),
        };
        let resource_name = match resource_name {
            Some(r) => r.to_string(),
            None => format!("{}/{}", ICUData::ICU_BASE_NAME, item_path),
        };
        let is = ICUData::get_stream_loader_required(&loader, &resource_name, required)?;
        let Some(mut is) = is else {
            return Ok(None);
        };
        match Self::get_byte_buffer_from_input_stream_and_close_stream(&mut is) {
            Ok(buffer) => Ok(Some(buffer)),
            Err(e) if e.is_io_exception() => Err(ICUUncheckedIOException::with_cause(e).error()),
            Err(e) => Err(e),
        }
    }

    /// `getDataFromFile(String itemPath)` (private).
    fn get_data_from_file(item_path: &str) -> Result<Option<ByteBuffer>> {
        for data_file in ICU_DATA_FILES.iter() {
            if let Some(data) = data_file.get_data(item_path)? {
                return Ok(Some(data));
            }
        }
        Ok(None)
    }

    /// `mapFile(File path)` (private): the file's bytes as a big-endian
    /// buffer, `None` (after printing the exception to `System.err`) when
    /// it cannot be read.
    ///
    /// **Divergence.** Java maps the file read-only
    /// (`FileChannel.map(READ_ONLY, 0, size)`); the port reads the file
    /// into memory, the crate forbidding the `unsafe` a memory map needs.
    /// The buffer's contents, capacity, limit, position and order are the
    /// same; only a later change to the file on disk, which a mapping
    /// would reflect, is not seen.
    fn map_file(path: &Path) -> Option<ByteBuffer> {
        match std::fs::read(path) {
            Ok(bytes) => Some(ByteBuffer::wrap(bytes)),
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    eprintln!(
                        "java.io.FileNotFoundException: {} (No such file or directory)",
                        path.display()
                    );
                } else {
                    eprintln!("java.io.IOException: {e}");
                }
                None
            }
        }
    }

    /// `addBaseNamesInFileFolder(String folder, String suffix, Set<String>
    /// names)`: the base names of the data items of the data path in
    /// `folder` that end with `suffix`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` for a malformed package table.
    pub fn add_base_names_in_file_folder(folder: &str, suffix: &str, names: &mut HashSet<String>) {
        for data_file in ICU_DATA_FILES.iter() {
            data_file.add_base_names_in_folder(folder, suffix, names);
        }
    }

    /// `readHeaderAndDataVersion(ByteBuffer, int dataFormat, Authenticate)`:
    /// [`read_header`](Self::read_header), the data version as a
    /// [`VersionInfo`].
    ///
    /// # Errors
    ///
    /// As [`read_header`](Self::read_header).
    pub fn read_header_and_data_version(
        bytes: &mut ByteBuffer,
        data_format: i32,
        authenticate: Option<Authenticate<'_>>,
    ) -> Result<VersionInfo> {
        Ok(Self::get_version_info_from_compact_int(Self::read_header(
            bytes,
            data_format,
            authenticate,
        )?))
    }

    /// `readHeader(ByteBuffer bytes, int dataFormat, Authenticate
    /// authenticate)`: checks the ICU data header, sets the buffer's byte
    /// order from it, moves the position past it and returns the data
    /// version as a compact int.
    ///
    /// # Errors
    ///
    /// `IOException` with ICU's messages when the magic number, the header
    /// or the data format does not match or the format version is not
    /// acceptable.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` for a buffer shorter than the header.
    pub fn read_header(
        bytes: &mut ByteBuffer,
        data_format: i32,
        authenticate: Option<Authenticate<'_>>,
    ) -> Result<i32> {
        debug_assert!(bytes.position() == 0);
        let magic1 = bytes.get_at(2);
        let magic2 = bytes.get_at(3);
        if magic1 != Self::MAGIC1 || magic2 != Self::MAGIC2 {
            return Err(io_exception(Self::MAGIC_NUMBER_AUTHENTICATION_FAILED));
        }

        let is_big_endian = bytes.get_at(8);
        let charset_family = bytes.get_at(9);
        let sizeof_u_char = bytes.get_at(10);
        if !(0..=1).contains(&is_big_endian)
            || charset_family != Self::CHAR_SET
            || sizeof_u_char != Self::CHAR_SIZE
        {
            return Err(io_exception(Self::HEADER_AUTHENTICATION_FAILED));
        }
        bytes.set_order(if is_big_endian != 0 {
            ByteOrder::BigEndian
        } else {
            ByteOrder::LittleEndian
        });

        let header_size = bytes.get_char_at(0) as i32;
        let sizeof_u_data_info = bytes.get_char_at(4) as i32;
        if sizeof_u_data_info < 20 || header_size < (sizeof_u_data_info + 4) {
            return Err(io_exception("Internal Error: Header size error"));
        }
        // TODO: Change Authenticate to take int major, int minor, int milli, int micro
        // to avoid array allocation.
        let format_version: [u8; 4] = [
            bytes.get_at(16) as u8,
            bytes.get_at(17) as u8,
            bytes.get_at(18) as u8,
            bytes.get_at(19) as u8,
        ];
        if bytes.get_at(12) != (data_format >> 24) as i8
            || bytes.get_at(13) != (data_format >> 16) as i8
            || bytes.get_at(14) != (data_format >> 8) as i8
            || bytes.get_at(15) != data_format as i8
            || authenticate.is_some_and(|a| !a(&format_version))
        {
            return Err(io_exception(format!(
                "{}; data format {:02x}{:02x}{:02x}{:02x}, format version {}.{}.{}.{}",
                Self::HEADER_AUTHENTICATION_FAILED,
                bytes.get_at(12) as u8,
                bytes.get_at(13) as u8,
                bytes.get_at(14) as u8,
                bytes.get_at(15) as u8,
                format_version[0],
                format_version[1],
                format_version[2],
                format_version[3]
            )));
        }

        bytes.set_position(header_size)?;
        Ok(((bytes.get_at(20) as i32) << 24)
            | ((bytes.get_at(21) as i32 & 0xff) << 16)
            | ((bytes.get_at(22) as i32 & 0xff) << 8)
            | (bytes.get_at(23) as i32 & 0xff))
    }

    /// `writeHeader(int dataFormat, int formatVersion, int dataVersion,
    /// DataOutputStream dos)`: writes a 32-byte big-endian ICU data header
    /// and returns its length.
    ///
    /// # Errors
    ///
    /// The `IOException` of the stream.
    pub fn write_header(
        data_format: i32,
        format_version: i32,
        data_version: i32,
        dos: &mut DataOutputStream<'_>,
    ) -> Result<i32> {
        // ucmndata.h MappedData
        dos.write_char(32)?; // headerSize
        dos.write_byte(Self::MAGIC1 as i32)?;
        dos.write_byte(Self::MAGIC2 as i32)?;
        // unicode/udata.h UDataInfo
        dos.write_char(20)?; // sizeof(UDataInfo)
        dos.write_char(0)?; // reservedWord
        dos.write_byte(1)?; // isBigEndian
        dos.write_byte(Self::CHAR_SET as i32)?; // charsetFamily
        dos.write_byte(Self::CHAR_SIZE as i32)?; // sizeofUChar
        dos.write_byte(0)?; // reservedByte
        dos.write_int(data_format)?;
        dos.write_int(format_version)?;
        dos.write_int(data_version)?;
        // 8 bytes padding for 32 bytes headerSize (multiple of 16).
        dos.write_long(0)?;
        debug_assert!(dos.size() == 32);
        Ok(32)
    }

    /// `skipBytes(ByteBuffer bytes, int skipLength)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the new position is past the limit.
    pub fn skip_bytes(bytes: &mut ByteBuffer, skip_length: i32) -> Result<()> {
        if skip_length > 0 {
            let p = bytes.position();
            bytes.set_position(p.wrapping_add(skip_length))?;
        }
        Ok(())
    }

    /// `getBytes(ByteBuffer bytes, int length, int additionalSkipLength)`.
    ///
    /// # Errors
    ///
    /// `BufferUnderflowException` or `IllegalArgumentException` from the
    /// buffer.
    pub fn get_bytes(bytes: &mut ByteBuffer, length: i32, additional_skip_length: i32) -> Result<Vec<i8>> {
        let mut dest = vec![0u8; length as usize];
        bytes.get_bytes(&mut dest)?;
        if additional_skip_length > 0 {
            Self::skip_bytes(bytes, additional_skip_length)?;
        }
        Ok(dest.into_iter().map(|b| b as i8).collect())
    }

    /// `getString(ByteBuffer bytes, int length, int additionalSkipLength)`:
    /// `length` UTF-16 units from the position, in the buffer's order.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` from the skip.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` when fewer units remain
    /// (`CharBuffer.subSequence`).
    pub fn get_string(bytes: &mut ByteBuffer, length: i32, additional_skip_length: i32) -> Result<Vec<u16>> {
        let available = bytes.remaining() / 2;
        if length < 0 || length > available {
            index_out_of_bounds(Some(format!(
                "Range [0, 0 + {length}) out of bounds for length {available}"
            )));
        }
        let mut s = vec![0u16; length as usize];
        bytes.view_chars(&mut s)?;
        Self::skip_bytes(bytes, length.wrapping_mul(2).wrapping_add(additional_skip_length))?;
        Ok(s)
    }

    /// `getChars(ByteBuffer bytes, int length, int additionalSkipLength)`.
    ///
    /// # Errors
    ///
    /// `BufferUnderflowException` or `IllegalArgumentException` from the
    /// buffer.
    pub fn get_chars(bytes: &mut ByteBuffer, length: i32, additional_skip_length: i32) -> Result<Vec<u16>> {
        let mut dest = vec![0u16; length as usize];
        bytes.view_chars(&mut dest)?;
        Self::skip_bytes(bytes, length.wrapping_mul(2).wrapping_add(additional_skip_length))?;
        Ok(dest)
    }

    /// `getShorts(ByteBuffer bytes, int length, int additionalSkipLength)`.
    ///
    /// # Errors
    ///
    /// `BufferUnderflowException` or `IllegalArgumentException` from the
    /// buffer.
    pub fn get_shorts(bytes: &mut ByteBuffer, length: i32, additional_skip_length: i32) -> Result<Vec<i16>> {
        let mut dest = vec![0i16; length as usize];
        bytes.view_shorts(&mut dest)?;
        Self::skip_bytes(bytes, length.wrapping_mul(2).wrapping_add(additional_skip_length))?;
        Ok(dest)
    }

    /// `getInts(ByteBuffer bytes, int length, int additionalSkipLength)`.
    ///
    /// # Errors
    ///
    /// `BufferUnderflowException` or `IllegalArgumentException` from the
    /// buffer.
    pub fn get_ints(bytes: &mut ByteBuffer, length: i32, additional_skip_length: i32) -> Result<Vec<i32>> {
        let mut dest = vec![0i32; length as usize];
        bytes.view_ints(&mut dest)?;
        Self::skip_bytes(bytes, length.wrapping_mul(4).wrapping_add(additional_skip_length))?;
        Ok(dest)
    }

    /// `getLongs(ByteBuffer bytes, int length, int additionalSkipLength)`.
    ///
    /// # Errors
    ///
    /// `BufferUnderflowException` or `IllegalArgumentException` from the
    /// buffer.
    pub fn get_longs(bytes: &mut ByteBuffer, length: i32, additional_skip_length: i32) -> Result<Vec<i64>> {
        let mut dest = vec![0i64; length as usize];
        bytes.view_longs(&mut dest)?;
        Self::skip_bytes(bytes, length.wrapping_mul(8).wrapping_add(additional_skip_length))?;
        Ok(dest)
    }

    /// `sliceWithOrder(ByteBuffer bytes)`: `slice()` keeping the byte order.
    pub fn slice_with_order(bytes: &ByteBuffer) -> ByteBuffer {
        let mut b = bytes.slice();
        b.set_order(bytes.order());
        b
    }

    /// `getByteBufferFromInputStreamAndCloseStream(InputStream is)`: reads
    /// the whole stream into a heap buffer and closes the stream.
    ///
    /// # Errors
    ///
    /// The `IOException` of the stream.
    pub fn get_byte_buffer_from_input_stream_and_close_stream(
        is: &mut dyn InputStream,
    ) -> Result<ByteBuffer> {
        let result = (|| -> Result<ByteBuffer> {
            // is.available() may return 0, or 1, or the total number of bytes in the stream,
            // or some other number.
            // Do not try to use is.available() == 0 to find the end of the stream!
            let avail = is.available()?;
            let mut bytes: Vec<u8> = if avail > 32 {
                // There are more bytes available than just the ICU data header length.
                // With luck, it is the total number of bytes.
                vec![0u8; avail as usize]
            } else {
                vec![0u8; 128] // empty .res files are even smaller
            };
            // Call is.read(...) until one returns a negative value.
            let mut length = 0usize;
            loop {
                if length < bytes.len() {
                    let cap = bytes.len();
                    let num_read = is.read(&mut bytes, length, cap - length)?;
                    if num_read < 0 {
                        break; // end of stream
                    }
                    length += num_read as usize;
                } else {
                    // See if we are at the end of the stream before we grow the array.
                    let next_byte = is.read_byte()?;
                    if next_byte < 0 {
                        break;
                    }
                    let mut capacity = 2 * bytes.len();
                    if capacity < 128 {
                        capacity = 128;
                    } else if capacity < 0x4000 {
                        capacity *= 2; // Grow faster until we reach 16kB.
                    }
                    bytes.resize(capacity, 0);
                    bytes[length] = next_byte as u8;
                    length += 1;
                }
            }
            Ok(ByteBuffer::wrap_range(bytes, 0, length as i32))
        })();
        let closed = is.close();
        let buffer = result?;
        closed?;
        Ok(buffer)
    }

    /// `getVersionInfoFromCompactInt(int version)`.
    pub fn get_version_info_from_compact_int(version: i32) -> VersionInfo {
        VersionInfo::get_instance4(
            ((version as u32) >> 24) as i32,
            (version >> 16) & 0xff,
            (version >> 8) & 0xff,
            version & 0xff,
        )
        .expect("every byte of a compact version is in 0..=255")
    }

    /// `getVersionByteArrayFromCompactInt(int version)`.
    pub fn get_version_byte_array_from_compact_int(version: i32) -> [i8; 4] {
        [
            (version >> 24) as i8,
            (version >> 16) as i8,
            (version >> 8) as i8,
            version as i8,
        ]
    }
}

/// `String.trim()`: strips the leading and trailing characters `<= ' '`.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| (c as u32) <= 0x20)
}
