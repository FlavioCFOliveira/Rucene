//! JVM introspection ported from `org.apache.lucene.util`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`HotspotVMOptions`] | `HotspotVMOptions` |
//! | [`MethodClass`] / [`VirtualMethod`] | `VirtualMethod<C>` |
//!
//! Both classes exist to ask the running JVM questions about itself. Rust has
//! neither a JVM nor runtime reflection, so each is ported to the construct
//! that answers the same question here; the divergences are stated on the
//! items themselves.

#![deny(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use crate::error::{LuceneError, Result};

// ---------------------------------------------------------------------------
// HotspotVMOptions
// ---------------------------------------------------------------------------

/// Access to HotSpot VM options.
///
/// Port of the package-private `org.apache.lucene.util.HotspotVMOptions`,
/// public here because Rust has no package visibility between sibling modules.
///
/// Java reflects on `com.sun.management.HotSpotDiagnosticMXBean` and answers
/// `getVMOption(name).getValue()`, or `Optional.empty()` when the option does
/// not exist on that VM. Lucene reads six options through it: `UseFMA`,
/// `UseAVX`, `UseXmmI2F`, `UseSVE` and `MaxVectorSize` in `Constants`
/// (`Constants.java:73-106`), and `UseCompressedOops` and
/// `ObjectAlignmentInBytes` in `RamUsageEstimator`, which reads them only when
/// `IS_HOTSPOT_VM` is true.
///
/// [`get`](Self::get) answers the five CPU options of `Constants` with the
/// values HotSpot's ergonomics give them on the running CPU, computed by the
/// rules of OpenJDK 25 (`VM_Version::get_processor_features` in
/// `src/hotspot/cpu/x86/vm_version_x86.cpp` and
/// `src/hotspot/cpu/aarch64/vm_version_aarch64.cpp`, read as specification),
/// so that `Constants.HAS_FMA`, `HAS_AVX2`, `HAS_SVE`, `HAS_SSE4A`,
/// `MAX_VECTOR_SIZE` and the `HAS_FAST_*_FMA` choices they decide equal those
/// of a default JVM on the same machine. An option that does not exist on the
/// architecture (`UseAVX` and `UseXmmI2F` off x86, `UseSVE` off aarch64) is
/// absent, as `getVMOption` throws for it on a real VM.
///
/// # Overriding an option (`-XX:`)
///
/// A JVM option set on the command line (`-XX:+UseFMA`, `-XX:UseAVX=2`,
/// `-XX:MaxVectorSize=16`) has its equivalent in the environment variable
/// `XX:<name>` holding the value `getVMOption` would print: `XX:UseFMA=false`,
/// `XX:UseAVX=2`, `XX:MaxVectorSize=16`. This mirrors the port's form of a
/// system property — `-Dlucene.useScalarFMA=true` is the environment variable
/// `lucene.useScalarFMA=true` (`Constants.getSysProp`). An override goes
/// through the same ergonomics a command-line flag does: `XX:UseAVX=3` on a CPU
/// without AVX-512 yields 2, `XX:UseFMA=true` on a CPU without FMA yields
/// `false`, `XX:MaxVectorSize` is clamped to the supported range. A value
/// HotSpot would refuse to start with (not `true`/`false`, not an integer, out
/// of the option's range, a non-power-of-two SVE vector size) is ignored with a
/// logged warning. The options are computed once, on first use, as a JVM fixes
/// them at start-up.
///
/// **Divergences from Lucene 10.5.0, declared.**
///
/// * [`is_hotspot_vm`](Self::is_hotspot_vm) stays `false`, and the
///   non-HotSpot warning is still logged: a Rust binary is not a HotSpot VM,
///   and the other consumers of that flag (`RamUsageEstimator`'s object-layout
///   options, the Panama vectorization provider gate) describe JVM internals
///   with no counterpart here. Only the CPU options above are answered.
/// * `UseJVMCICompiler` is absent, as on a JDK built without JVMCI (the JDK 25
///   oracle used by this port); on a JVMCI build its default is `false`, so
///   `Constants.IS_JVMCI_VM` is `false` either way.
/// * HotSpot's signal-handler probe of whether the OS preserves YMM/ZMM state
///   (`os_supports_avx_vectors`) is answered by the XCR0 state check
///   `is_x86_feature_detected!` performs, which is the OS enabling that state.
/// * Intel's AVX10.1/512 path to `CPU_AVX512F` is covered by `avx512f`
///   detection, since AVX10/512 processors also enumerate the AVX-512 CPUID
///   bits (Intel AVX10 Architecture Specification); `avx10.1` detection is
///   unstable in Rust.
/// * On aarch64 the SVE vector length is read with `prctl(PR_SVE_GET_VL)` as
///   HotSpot does, but never changed: an `XX:MaxVectorSize` override larger
///   than the current length is clamped to it, where HotSpot first tries to
///   raise the process vector length with `PR_SVE_SET_VL`.
/// * Architectures other than x86_64 and aarch64 answer no option, as on a
///   non-HotSpot VM.
pub struct HotspotVMOptions;

/// Emits the warning at most once, as a static initialiser would.
static WARNED: OnceLock<()> = OnceLock::new();

/// The CPU options of [`HotspotVMOptions`], computed once.
static VM_OPTIONS: OnceLock<CpuVmOptions> = OnceLock::new();

impl HotspotVMOptions {
    /// The warning Java logs when the HotSpot diagnostic bean is unavailable.
    pub const NOT_HOTSPOT_WARNING: &'static str =
        "Lucene cannot optimize algorithms or calculate object sizes for JVMs that are not based \
         on Hotspot or a compatible implementation.";

    /// Whether the process runs on a HotSpot-compatible VM.
    ///
    /// Equivalent to `HotspotVMOptions.IS_HOTSPOT_VM`; always `false` here
    /// (see the type documentation).
    pub fn is_hotspot_vm() -> bool {
        Self::warn_once();
        false
    }

    /// Returns the value of a `-XX:` VM option.
    ///
    /// Equivalent to `HotspotVMOptions.get(String)`: the value HotSpot's
    /// ergonomics give `UseFMA`, `UseAVX`, `UseXmmI2F`, `UseSVE` or
    /// `MaxVectorSize` on the running CPU, after any `XX:<name>` override (see
    /// the type documentation), rendered as `VMOption.getValue()` renders it;
    /// `None` for every other option and for an option the architecture does
    /// not have.
    ///
    /// `Objects.requireNonNull(name, "name")` has no counterpart: a `&str` is
    /// never `null`, and an empty name is a valid, unknown option.
    pub fn get(name: &str) -> Option<String> {
        Self::warn_once();
        VM_OPTIONS
            .get_or_init(|| CpuVmOptions::detect(&|flag| env_override(flag)))
            .get(name)
    }

    fn warn_once() {
        WARNED.get_or_init(|| {
            log::warn!("{}", Self::NOT_HOTSPOT_WARNING);
        });
    }
}

/// Reads the `XX:<name>` environment variable standing for `-XX:<name>`.
fn env_override(name: &str) -> Option<String> {
    std::env::var(format!("XX:{name}")).ok()
}

/// Parses an override of a boolean option, as HotSpot's flag parser accepts
/// `true`/`false` (`-XX:+Name`/`-XX:-Name` are written out as these values).
fn parse_bool_override(name: &str, value: Option<String>) -> Option<bool> {
    let value = value?;
    match value.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => {
            log::warn!("ignoring XX:{name}={value}: not a boolean");
            None
        }
    }
}

/// Parses an override of an integer option within HotSpot's declared range.
fn parse_int_override(name: &str, value: Option<String>, min: i64, max: i64) -> Option<i64> {
    let value = value?;
    match value.parse::<i64>() {
        Ok(v) if (min..=max).contains(&v) => Some(v),
        _ => {
            log::warn!("ignoring XX:{name}={value}: not an integer in [{min}, {max}]");
            None
        }
    }
}

/// The values of the CPU options Lucene reads, as HotSpot sets them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CpuVmOptions {
    use_fma: Option<bool>,
    use_avx: Option<i64>,
    use_xmm_i2f: Option<bool>,
    use_sve: Option<i64>,
    max_vector_size: Option<i64>,
}

impl CpuVmOptions {
    /// The value of `name`, rendered as `VMOption.getValue()` renders it.
    pub(crate) fn get(&self, name: &str) -> Option<String> {
        match name {
            "UseFMA" => self.use_fma.map(|v| v.to_string()),
            "UseAVX" => self.use_avx.map(|v| v.to_string()),
            "UseXmmI2F" => self.use_xmm_i2f.map(|v| v.to_string()),
            "UseSVE" => self.use_sve.map(|v| v.to_string()),
            "MaxVectorSize" => self.max_vector_size.map(|v| v.to_string()),
            _ => None,
        }
    }

    /// The options on the running CPU, with `overrides` standing for the
    /// `-XX:` command line.
    #[allow(unused_variables)]
    fn detect(overrides: &dyn Fn(&str) -> Option<String>) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            Self::x86(&X86Cpu::detect(), overrides)
        }
        #[cfg(target_arch = "aarch64")]
        {
            Self::aarch64(&Aarch64Cpu::detect(), overrides)
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            Self::default()
        }
    }

    #[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
    /// HotSpot's x86 ergonomics for the options Lucene reads
    /// (`vm_version_x86.cpp`, `VM_Version::get_processor_features`).
    pub(crate) fn x86(cpu: &X86Cpu, overrides: &dyn Fn(&str) -> Option<String>) -> Self {
        // UseSSE (default 99) is limited to what the CPU supports; x86_64
        // always has SSE2.
        let use_sse = if cpu.sse4_1 {
            4
        } else if cpu.sse3 {
            3
        } else {
            2
        };

        // UseAVX: default 3, range(0, 3); ZX cpus of family 6 and 7 assign 0.
        let avx_override = parse_int_override("UseAVX", overrides("UseAVX"), 0, 3);
        let avx_is_default = avx_override.is_none();
        let mut use_avx = avx_override.unwrap_or(3);
        if cpu.is_zx() && (cpu.family == 6 || cpu.family == 7) {
            use_avx = 0;
        }
        let use_avx_limit = if use_avx > 0 {
            if use_sse < 4 {
                0
            } else if use_avx > 2 && cpu.avx512f {
                3
            } else if use_avx > 1 && cpu.avx2 {
                2
            } else if cpu.avx {
                1
            } else {
                0
            }
        } else {
            0
        };
        if avx_is_default {
            // Don't use AVX-512 on older Skylakes unless explicitly requested.
            use_avx = if use_avx_limit > 2 && cpu.is_intel_skylake() && cpu.stepping < 5 {
                2
            } else {
                use_avx_limit
            };
        }
        if use_avx > use_avx_limit {
            use_avx = use_avx_limit;
        }
        // Features cleared by a lower UseAVX/UseSSE.
        let avx512f = cpu.avx512f && use_avx >= 3;
        let avx = cpu.avx && use_avx >= 1;
        let sse4a = cpu.sse4a && use_sse >= 3;

        // UseFMA: supports_fma() is CPU_FMA && supports_avx().
        let use_fma = if cpu.fma && avx {
            parse_bool_override("UseFMA", overrides("UseFMA")).unwrap_or(true)
        } else {
            false
        };

        // MaxVectorSize (C2): default 64, range(0, max_jint).
        let max_vector_size_limit = if use_avx == 0 || !cpu.os_supports_avx_vectors(avx512f, avx) {
            16
        } else if use_avx <= 2 {
            32
        } else {
            64
        };
        let mut max_vector_size = match parse_int_override(
            "MaxVectorSize",
            overrides("MaxVectorSize"),
            0,
            i64::from(i32::MAX),
        ) {
            None => max_vector_size_limit,
            Some(v) if v < 4 => 4,
            Some(v) if v > max_vector_size_limit => max_vector_size_limit,
            Some(v) if !(v as u64).is_power_of_two() => max_vector_size_limit,
            Some(v) => v,
        };

        // UseXmmI2F: default false; AMD-family cpus set it from SSE4a.
        let xmm_override = parse_bool_override("UseXmmI2F", overrides("UseXmmI2F"));
        let use_xmm_i2f = if cpu.is_amd_family() {
            // Limit vectors size to 16 bytes on AMD cpus < 17h.
            if cpu.family < 0x17 && max_vector_size > 16 {
                max_vector_size = 16;
            }
            xmm_override.unwrap_or(sse4a)
        } else {
            xmm_override.unwrap_or(false)
        };

        Self {
            use_fma: Some(use_fma),
            use_avx: Some(use_avx),
            use_xmm_i2f: Some(use_xmm_i2f),
            use_sve: None,
            max_vector_size: Some(max_vector_size),
        }
    }

    #[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
    /// HotSpot's aarch64 ergonomics for the options Lucene reads
    /// (`vm_version_aarch64.cpp`, `VM_Version::initialize`).
    pub(crate) fn aarch64(cpu: &Aarch64Cpu, overrides: &dyn Fn(&str) -> Option<String>) -> Self {
        const SVE_VL_MIN: i64 = 16;
        const NEON_VL: i64 = 16;

        // UseFMA: default false, set to true unless given.
        let use_fma = parse_bool_override("UseFMA", overrides("UseFMA")).unwrap_or(true);

        // UseSVE: default 0, range(0, 2).
        let sve_override = parse_int_override("UseSVE", overrides("UseSVE"), 0, 2);
        let mut use_sve = if cpu.sve2 {
            sve_override.unwrap_or(2)
        } else if cpu.sve {
            sve_override.unwrap_or(1).min(1)
        } else {
            0
        };
        let mut initial_vl = 0;
        if use_sve > 0 {
            let vl = cpu.sve_vector_length;
            if vl <= 0 || vl % SVE_VL_MIN != 0 || !(vl as u64).is_power_of_two() {
                use_sve = 0;
            } else {
                initial_vl = vl;
            }
        }

        let mvs_override = overrides("MaxVectorSize").and_then(|v| {
            let parsed = parse_int_override("MaxVectorSize", Some(v), 0, i64::from(i32::MAX))?;
            // HotSpot exits on a size that is not a power of two (both the
            // SVE and the NEON branch); ignore it.
            if !(parsed as u64).is_power_of_two() {
                log::warn!("ignoring XX:MaxVectorSize={parsed}: unsupported vector size");
                None
            } else {
                Some(parsed)
            }
        });
        let mut max_vector_size = 0;
        if use_sve > 0 {
            match mvs_override {
                None => max_vector_size = initial_vl,
                Some(v) if v < SVE_VL_MIN => use_sve = 0,
                // The vector length is not raised (see the type documentation).
                Some(v) => max_vector_size = v.min(initial_vl),
            }
        }
        if use_sve == 0 {
            max_vector_size = match mvs_override {
                None => NEON_VL,
                Some(v) if v < 8 => 8,
                Some(v) if v > NEON_VL => NEON_VL,
                Some(v) => v,
            };
        }

        Self {
            use_fma: Some(use_fma),
            use_avx: None,
            use_xmm_i2f: None,
            use_sve: Some(use_sve),
            max_vector_size: Some(max_vector_size),
        }
    }
}

/// The x86 CPU facts HotSpot's ergonomics consult (`VM_Version::CpuidInfo`).
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct X86Cpu {
    /// `CPUID.0:EBX`, the first four bytes of the vendor string.
    pub(crate) vendor_ebx: u32,
    /// `extended_cpu_family()`: family plus extended family.
    pub(crate) family: u32,
    /// `extended_cpu_model()`: model with the extended model as high nibble.
    pub(crate) model: u32,
    /// `cpu_stepping()`.
    pub(crate) stepping: u32,
    pub(crate) sse3: bool,
    pub(crate) sse4_1: bool,
    /// AVX with the OS saving YMM state (`CPU_AVX`).
    pub(crate) avx: bool,
    pub(crate) avx2: bool,
    /// AVX-512F with the OS saving ZMM state (`CPU_AVX512F`).
    pub(crate) avx512f: bool,
    /// `CPUID.1:ECX.FMA` (`CPU_FMA`).
    pub(crate) fma: bool,
    /// `CPUID.80000001h:ECX.SSE4A`, recorded only on AMD-family cpus
    /// (`CPU_SSE4A`).
    pub(crate) sse4a: bool,
}

#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
impl X86Cpu {
    const VENDOR_AMD: u32 = 0x6874_7541; // "Auth"
    const VENDOR_HYGON: u32 = 0x6F67_7948; // "Hygo"
    const VENDOR_INTEL: u32 = 0x756E_6547; // "Genu"
    const VENDOR_CENTAUR: u32 = 0x746E_6543; // "Cent"
    const VENDOR_SHANGHAI: u32 = 0x6853_2020; // "  Sh"
    const FAMILY_INTEL_CORE: u32 = 6;
    const MODEL_SKYLAKE: u32 = 0x55;

    fn is_amd_family(&self) -> bool {
        self.vendor_ebx == Self::VENDOR_AMD || self.vendor_ebx == Self::VENDOR_HYGON
    }

    fn is_zx(&self) -> bool {
        self.vendor_ebx == Self::VENDOR_CENTAUR || self.vendor_ebx == Self::VENDOR_SHANGHAI
    }

    fn is_intel_skylake(&self) -> bool {
        self.vendor_ebx == Self::VENDOR_INTEL
            && self.family == Self::FAMILY_INTEL_CORE
            && self.model == Self::MODEL_SKYLAKE
    }

    /// `os_supports_avx_vectors()` once the features above are filtered by
    /// `UseAVX`: true when the OS saves the widest enabled register state.
    fn os_supports_avx_vectors(&self, avx512f: bool, avx: bool) -> bool {
        avx512f || avx
    }

    /// The facts of the running CPU.
    #[cfg(target_arch = "x86_64")]
    fn detect() -> Self {
        use std::arch::is_x86_feature_detected;
        let vendor_ebx = cpuid(0).ebx;
        let eax = cpuid(1).eax;
        let (family, ext_family) = ((eax >> 8) & 0xF, (eax >> 20) & 0xFF);
        let (model, ext_model) = ((eax >> 4) & 0xF, (eax >> 16) & 0xF);
        let mut cpu = Self {
            vendor_ebx,
            family: family + ext_family,
            model: model | (ext_model << 4),
            stepping: eax & 0xF,
            sse3: is_x86_feature_detected!("sse3"),
            sse4_1: is_x86_feature_detected!("sse4.1"),
            avx: is_x86_feature_detected!("avx"),
            avx2: is_x86_feature_detected!("avx2"),
            avx512f: is_x86_feature_detected!("avx512f"),
            fma: is_x86_feature_detected!("fma"),
            sse4a: false,
        };
        cpu.sse4a = cpu.is_amd_family() && is_x86_feature_detected!("sse4a");
        cpu
    }
}

/// `CPUID` with `EAX = leaf`.
#[cfg(target_arch = "x86_64")]
#[allow(unsafe_code, unused_unsafe)]
fn cpuid(leaf: u32) -> std::arch::x86_64::CpuidResult {
    // SAFETY: every x86_64 processor implements CPUID, and leaves 0 and 1
    // are defined on all of them; the intrinsic has no other precondition
    // (it is a safe function in recent Rust releases, unsafe at the 1.89
    // MSRV, hence `unused_unsafe`).
    unsafe { std::arch::x86_64::__cpuid(leaf) }
}

/// The aarch64 CPU facts HotSpot's ergonomics consult.
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Aarch64Cpu {
    /// `HWCAP_SVE`.
    pub(crate) sve: bool,
    /// `HWCAP2_SVE2`.
    pub(crate) sve2: bool,
    /// `get_current_sve_vector_length()`: the raw `prctl(PR_SVE_GET_VL)`
    /// result, `-1` when it cannot be read.
    pub(crate) sve_vector_length: i64,
}

impl Aarch64Cpu {
    /// The facts of the running CPU.
    #[cfg(target_arch = "aarch64")]
    fn detect() -> Self {
        use std::arch::is_aarch64_feature_detected;
        let sve = is_aarch64_feature_detected!("sve");
        Self {
            sve,
            sve2: is_aarch64_feature_detected!("sve2"),
            sve_vector_length: if sve { sve_get_vl() } else { -1 },
        }
    }
}

/// `prctl(PR_SVE_GET_VL)`, as HotSpot's Linux `get_current_sve_vector_length`
/// reads it (unmasked).
#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
#[allow(unsafe_code)]
fn sve_get_vl() -> i64 {
    /// `PR_SVE_GET_VL` from `<linux/prctl.h>`.
    const PR_SVE_GET_VL: libc::c_int = 51;
    // SAFETY: PR_SVE_GET_VL takes no further argument, reads only the calling
    // thread's SVE configuration and returns it, or -1 with errno set.
    i64::from(unsafe { libc::prctl(PR_SVE_GET_VL) })
}

/// HotSpot's BSD port has no SVE vector length query (`-1`).
#[cfg(all(target_arch = "aarch64", not(target_os = "linux")))]
fn sve_get_vl() -> i64 {
    -1
}

// ---------------------------------------------------------------------------
// VirtualMethod
// ---------------------------------------------------------------------------

/// A node in a class hierarchy, carrying the methods it declares.
///
/// Rust has neither inheritance nor reflection, so [`VirtualMethod`] cannot
/// interrogate real types. This models the two things it inspects — the
/// superclass chain and the set of methods each class declares — so that
/// Lucene's algorithm can be reproduced on it verbatim. Classes are compared by
/// identity, matching Java's `==` on `Class` objects.
#[derive(Debug)]
pub struct MethodClass {
    name: String,
    super_class: Option<Arc<MethodClass>>,
    declared_methods: HashSet<String>,
}

impl MethodClass {
    /// Creates a root class declaring `declared_methods`.
    pub fn new<I, S>(name: impl Into<String>, declared_methods: I) -> Arc<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Arc::new(Self {
            name: name.into(),
            super_class: None,
            declared_methods: declared_methods.into_iter().map(Into::into).collect(),
        })
    }

    /// Creates a class extending `super_class` and declaring
    /// `declared_methods`.
    pub fn extending<I, S>(
        name: impl Into<String>,
        super_class: Arc<MethodClass>,
        declared_methods: I,
    ) -> Arc<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Arc::new(Self {
            name: name.into(),
            super_class: Some(super_class),
            declared_methods: declared_methods.into_iter().map(Into::into).collect(),
        })
    }

    /// Returns this class's name.
    pub fn get_name(&self) -> &str {
        &self.name
    }

    /// Returns this class's superclass, if any.
    ///
    /// Equivalent to `Class.getSuperclass()`.
    pub fn get_superclass(&self) -> Option<&Arc<MethodClass>> {
        self.super_class.as_ref()
    }

    /// Returns whether this class declares `method`.
    ///
    /// Equivalent to `Class.getDeclaredMethod(String, Class...)` succeeding.
    pub fn has_declared_method(&self, method: &str) -> bool {
        self.declared_methods.contains(method)
    }

    /// Returns whether `subclazz` is this class or descends from it.
    ///
    /// Equivalent to `Class.isAssignableFrom(Class)`.
    pub fn is_assignable_from(self: &Arc<Self>, subclazz: &Arc<MethodClass>) -> bool {
        let mut current = Some(subclazz.clone());
        while let Some(c) = current {
            if Arc::ptr_eq(&c, self) {
                return true;
            }
            current = c.super_class.clone();
        }
        false
    }
}

/// The set of `(base class, method)` pairs already claimed, enforcing the
/// singleton rule. Java uses a synchronised `HashSet<Method>`.
static SINGLETON_SET: Mutex<Option<HashSet<(String, String)>>> = Mutex::new(None);

/// Measures how deeply a method is overridden below a base class.
///
/// Port of `org.apache.lucene.util.VirtualMethod`. Lucene uses it to decide
/// whether a subclass overrides a deprecated method, so that the correct
/// variant is called; see the class javadoc for why the instance must be a
/// singleton assigned to a static final member.
pub struct VirtualMethod {
    base_class: Arc<MethodClass>,
    method: String,
    /// Java memoises with a `ClassValue`; a map keyed by class identity is the
    /// direct equivalent.
    distance_of_class: Mutex<HashMap<usize, i32>>,
}

impl std::fmt::Debug for VirtualMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualMethod")
            .field("base_class", &self.base_class.name)
            .field("method", &self.method)
            .finish()
    }
}

impl VirtualMethod {
    /// Creates a `VirtualMethod` instance for `method` declared by
    /// `base_class`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when `base_class` does not
    /// declare `method`, and [`LuceneErrorKind::UnsupportedOperation`](crate::error::LuceneErrorKind::UnsupportedOperation) when an
    /// instance for the same pair already exists — both reproducing Lucene's
    /// messages, which enforce that instances are singletons.
    pub fn new(base_class: Arc<MethodClass>, method: impl Into<String>) -> Result<Self> {
        let method = method.into();
        if !base_class.has_declared_method(&method) {
            return Err(LuceneError::illegal_argument(format!(
                "{} has no such method: {method}",
                base_class.name
            )));
        }
        let key = (base_class.name.clone(), method.clone());
        {
            let mut guard = SINGLETON_SET
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let set = guard.get_or_insert_with(HashSet::new);
            if !set.insert(key) {
                return Err(LuceneError::unsupported_operation(
                    "VirtualMethod instances must be singletons and therefore assigned to static \
                     final members in the same class, they use as baseClass ctor param."
                        .to_string(),
                ));
            }
        }
        Ok(Self {
            base_class,
            method,
            distance_of_class: Mutex::new(HashMap::new()),
        })
    }

    /// Returns how many classes between `subclazz` and the base class override
    /// the method, memoising the answer.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when `subclazz` does not
    /// descend from the base class.
    pub fn get_implementation_distance(&self, subclazz: &Arc<MethodClass>) -> Result<i32> {
        let key = Arc::as_ptr(subclazz) as *const () as usize;
        if let Some(&d) = self
            .distance_of_class
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key)
        {
            return Ok(d);
        }
        let distance = self.reflect_implementation_distance(subclazz)?;
        self.distance_of_class
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, distance);
        Ok(distance)
    }

    /// Returns whether `subclazz` overrides the method.
    ///
    /// # Errors
    ///
    /// As [`VirtualMethod::get_implementation_distance`].
    pub fn is_overridden_as_of(&self, subclazz: &Arc<MethodClass>) -> Result<bool> {
        Ok(self.get_implementation_distance(subclazz)? > 0)
    }

    /// Computes the implementation distance without consulting the cache.
    ///
    /// Equivalent to the package-private
    /// `VirtualMethod.reflectImplementationDistance`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when `subclazz` does not
    /// descend from the base class.
    pub fn reflect_implementation_distance(&self, subclazz: &Arc<MethodClass>) -> Result<i32> {
        if !self.base_class.is_assignable_from(subclazz) {
            return Err(LuceneError::illegal_argument(format!(
                "{} is not a subclass of {}",
                subclazz.name, self.base_class.name
            )));
        }
        let mut overridden = false;
        let mut distance = 0i32;
        let mut clazz = Some(subclazz.clone());
        while let Some(current) = clazz {
            if Arc::ptr_eq(&current, &self.base_class) {
                break;
            }
            // Look the method up; mark as overridden on success.
            if !overridden && current.has_declared_method(&self.method) {
                overridden = true;
            }
            // Increment the distance once the method is known to be overridden.
            if overridden {
                distance += 1;
            }
            clazz = current.super_class.clone();
        }
        Ok(distance)
    }

    /// Compares the implementation distance of `clazz` under `m1` and `m2`.
    ///
    /// Equivalent to
    /// `VirtualMethod.compareImplementationDistance(Class, VirtualMethod, VirtualMethod)`.
    ///
    /// # Errors
    ///
    /// As [`VirtualMethod::get_implementation_distance`].
    pub fn compare_implementation_distance(
        clazz: &Arc<MethodClass>,
        m1: &VirtualMethod,
        m2: &VirtualMethod,
    ) -> Result<std::cmp::Ordering> {
        Ok(m1
            .get_implementation_distance(clazz)?
            .cmp(&m2.get_implementation_distance(clazz)?))
    }
}

#[cfg(test)]
mod hotspot_vm_options_tests {
    use super::*;

    /// No `-XX:` override.
    fn none(_: &str) -> Option<String> {
        None
    }

    /// A Zen 3 (family 19h) such as the oracle host: AVX2, FMA, SSE4a.
    fn zen3() -> X86Cpu {
        X86Cpu {
            vendor_ebx: X86Cpu::VENDOR_AMD,
            family: 0x19,
            model: 0x50,
            stepping: 0,
            sse3: true,
            sse4_1: true,
            avx: true,
            avx2: true,
            avx512f: false,
            fma: true,
            sse4a: true,
        }
    }

    fn values(options: &CpuVmOptions) -> [Option<String>; 5] {
        ["UseFMA", "UseAVX", "UseXmmI2F", "UseSVE", "MaxVectorSize"].map(|n| options.get(n))
    }

    fn some(values: [&str; 5]) -> [Option<String>; 5] {
        values.map(|v| (!v.is_empty()).then(|| v.to_string()))
    }

    #[test]
    fn x86_defaults_follow_hotspot_ergonomics() {
        // The oracle host: `-XX:+PrintFlagsFinal` reports UseFMA=true,
        // UseAVX=2, UseXmmI2F=true, MaxVectorSize=32.
        assert_eq!(
            values(&CpuVmOptions::x86(&zen3(), &none)),
            some(["true", "2", "true", "", "32"])
        );
        // AVX-512 Intel (Ice Lake server, model 0x6A).
        let icelake = X86Cpu {
            vendor_ebx: X86Cpu::VENDOR_INTEL,
            family: 6,
            model: 0x6A,
            avx512f: true,
            sse4a: false,
            ..zen3()
        };
        assert_eq!(
            values(&CpuVmOptions::x86(&icelake, &none)),
            some(["true", "3", "false", "", "64"])
        );
        // Skylake-SP before stepping 5 keeps AVX2 unless asked.
        let skylake = X86Cpu {
            model: 0x55,
            stepping: 4,
            ..icelake
        };
        assert_eq!(
            values(&CpuVmOptions::x86(&skylake, &none)),
            some(["true", "2", "false", "", "32"])
        );
        let explicit = |n: &str| (n == "UseAVX").then(|| "3".to_string());
        assert_eq!(
            values(&CpuVmOptions::x86(&skylake, &explicit)),
            some(["true", "3", "false", "", "64"])
        );
        // AMD before family 17h: vectors limited to 16 bytes.
        let bulldozer = X86Cpu {
            family: 0x15,
            avx2: false,
            ..zen3()
        };
        assert_eq!(
            values(&CpuVmOptions::x86(&bulldozer, &none)),
            some(["true", "1", "true", "", "16"])
        );
        // Zhaoxin family 7: AVX forced off, so no FMA.
        let zx = X86Cpu {
            vendor_ebx: X86Cpu::VENDOR_SHANGHAI,
            family: 7,
            sse4a: false,
            ..zen3()
        };
        assert_eq!(
            values(&CpuVmOptions::x86(&zx, &none)),
            some(["false", "0", "false", "", "16"])
        );
        // No SSE4.1: UseSSE=3 disables AVX.
        let old = X86Cpu {
            sse4_1: false,
            ..zen3()
        };
        assert_eq!(
            values(&CpuVmOptions::x86(&old, &none)),
            some(["false", "0", "true", "", "16"])
        );
    }

    #[test]
    fn x86_overrides_go_through_the_ergonomics() {
        let set = |pairs: &'static [(&'static str, &'static str)]| {
            move |n: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == n)
                    .map(|(_, v)| v.to_string())
            }
        };
        // -XX:UseAVX=3 without AVX-512 falls back to 2.
        assert_eq!(
            values(&CpuVmOptions::x86(&zen3(), &set(&[("UseAVX", "3")]))),
            some(["true", "2", "true", "", "32"])
        );
        // -XX:UseAVX=0 clears AVX and with it FMA.
        assert_eq!(
            values(&CpuVmOptions::x86(&zen3(), &set(&[("UseAVX", "0")]))),
            some(["false", "0", "true", "", "16"])
        );
        // -XX:-UseFMA, -XX:-UseXmmI2F, -XX:MaxVectorSize=16.
        assert_eq!(
            values(&CpuVmOptions::x86(
                &zen3(),
                &set(&[
                    ("UseFMA", "false"),
                    ("UseXmmI2F", "false"),
                    ("MaxVectorSize", "16")
                ])
            )),
            some(["false", "2", "false", "", "16"])
        );
        // MaxVectorSize: below 4 -> 4, above the limit or not a power of two
        // -> the limit.
        for (given, expected) in [("2", "4"), ("128", "32"), ("24", "32"), ("8", "8")] {
            let pairs: &'static [(&str, &str)] =
                Box::leak(vec![("MaxVectorSize", given)].into_boxed_slice());
            assert_eq!(
                CpuVmOptions::x86(&zen3(), &set(pairs)).get("MaxVectorSize"),
                Some(expected.to_string()),
                "MaxVectorSize={given}"
            );
        }
        // -XX:+UseFMA on a CPU without FMA stays false.
        let no_fma = X86Cpu {
            fma: false,
            ..zen3()
        };
        assert_eq!(
            CpuVmOptions::x86(&no_fma, &set(&[("UseFMA", "true")])).get("UseFMA"),
            Some("false".to_string())
        );
        // Values HotSpot refuses to start with are ignored.
        assert_eq!(
            values(&CpuVmOptions::x86(
                &zen3(),
                &set(&[("UseAVX", "7"), ("UseFMA", "yes")])
            )),
            some(["true", "2", "true", "", "32"])
        );
    }

    #[test]
    fn aarch64_defaults_and_overrides_follow_hotspot_ergonomics() {
        let neon = Aarch64Cpu {
            sve: false,
            sve2: false,
            sve_vector_length: -1,
        };
        assert_eq!(
            values(&CpuVmOptions::aarch64(&neon, &none)),
            some(["true", "", "", "0", "16"])
        );
        let sve2 = Aarch64Cpu {
            sve: true,
            sve2: true,
            sve_vector_length: 16,
        };
        assert_eq!(
            values(&CpuVmOptions::aarch64(&sve2, &none)),
            some(["true", "", "", "2", "16"])
        );
        let sve256 = Aarch64Cpu {
            sve: true,
            sve2: false,
            sve_vector_length: 32,
        };
        assert_eq!(
            values(&CpuVmOptions::aarch64(&sve256, &none)),
            some(["true", "", "", "1", "32"])
        );
        // An inherited-configuration flag in the prctl result disables SVE.
        let inherit = Aarch64Cpu {
            sve_vector_length: 32 | (1 << 17),
            ..sve256
        };
        assert_eq!(
            values(&CpuVmOptions::aarch64(&inherit, &none)),
            some(["true", "", "", "0", "16"])
        );
        // -XX:UseSVE=2 without SVE2 yields 1; -XX:MaxVectorSize=8 disables SVE.
        let over = |n: &str| match n {
            "UseSVE" => Some("2".to_string()),
            "UseFMA" => Some("false".to_string()),
            _ => None,
        };
        assert_eq!(
            values(&CpuVmOptions::aarch64(&sve256, &over)),
            some(["false", "", "", "1", "32"])
        );
        let small = |n: &str| (n == "MaxVectorSize").then(|| "8".to_string());
        assert_eq!(
            values(&CpuVmOptions::aarch64(&sve256, &small)),
            some(["true", "", "", "0", "8"])
        );
    }

    #[test]
    fn unknown_and_absent_options_are_none() {
        let options = CpuVmOptions::x86(&zen3(), &none);
        assert_eq!(options.get("UseSVE"), None);
        assert_eq!(options.get("UseJVMCICompiler"), None);
        assert_eq!(options.get(""), None);
        assert_eq!(HotspotVMOptions::get("UseJVMCICompiler"), None);
    }
}
