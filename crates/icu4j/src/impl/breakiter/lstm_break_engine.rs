// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.breakiter.LSTMBreakEngine`, with its nested types
//! `EmbeddingType`, `LSTMClass`, `LSTMData` and the vectorizers.
//!
//! ICU4J 77.1 ships no LSTM model data (`brkitr/root.res` names
//! `Thai_graphclust_model4_heavy` and `Burmese_graphclust_model5_heavy`,
//! absent from the jar), so [`create_data_for_script`] fails with
//! `MissingResourceException` and `RuleBasedBreakIterator` falls back to the
//! dictionary engines, as in Java (decision D3 on rmp task #640).

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use rucene::error::Result;
use rucene::util::java_collections::java_identity_hash_code_stand_in;
use rucene::util::java_math;
use rucene_analysis_common::jdk::character_iterator::{CharacterIterator, DONE};

use crate::jdk::{to_rust_string, units};
use crate::lang::u_character::UCharacter;
use crate::lang::u_property::UProperty;
use crate::lang::u_script::UScript;
use crate::r#impl::breakiter::dictionary_break_engine::{
    dictionary_find_breaks, DequeI, DictionaryBreakEngine, DictionaryBreakEngineBase,
};
use crate::r#impl::breakiter::language_break_engine::LanguageBreakEngine;
use crate::r#impl::character_iteration::set_index;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle;
use crate::text::break_iterator::{self, DONE as BI_DONE};
use crate::text::unicode_set::UnicodeSet;
use crate::util::u_resource_bundle::UResourceBundle;

/// The embedding type of a model.
///
/// Equivalent to the enum `LSTMBreakEngine.EmbeddingType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingType {
    /// `UNKNOWN`.
    Unknown,
    /// `CODE_POINTS`.
    CodePoints,
    /// `GRAPHEME_CLUSTER`.
    GraphemeCluster,
}

/// The classes the model assigns to each input element.
///
/// Equivalent to the enum `LSTMBreakEngine.LSTMClass`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LSTMClass {
    /// `BEGIN`.
    Begin,
    /// `INSIDE`.
    Inside,
    /// `END`.
    End,
    /// `SINGLE`.
    Single,
}

impl LSTMClass {
    /// `ordinal()`.
    pub fn ordinal(self) -> i32 {
        match self {
            LSTMClass::Begin => 0,
            LSTMClass::Inside => 1,
            LSTMClass::End => 2,
            LSTMClass::Single => 3,
        }
    }
}

/// `Float.intBitsToFloat` of a big-endian `int`: `ByteBuffer.wrap(bytes)
/// .order(BIG_ENDIAN).getFloat()` over the four bytes of `d`.
fn int_to_float(d: i32) -> f32 {
    f32::from_bits(d as u32)
}

/// `make2DArray(int[] data, int start, int d1, int d2)`.
fn make_2d_array(data: &[i32], mut start: usize, d1: usize, d2: usize) -> Vec<Vec<f32>> {
    let mut result = vec![vec![0f32; d2]; d1];
    for row in result.iter_mut() {
        for v in row.iter_mut() {
            *v = int_to_float(data[start]);
            start += 1;
        }
    }
    result
}

/// `make1DArray(int[] data, int start, int d1)`.
fn make_1d_array(data: &[i32], mut start: usize, d1: usize) -> Vec<f32> {
    let mut result = vec![0f32; d1];
    for v in result.iter_mut() {
        *v = int_to_float(data[start]);
        start += 1;
    }
    result
}

/// The weights of an LSTM model.
///
/// Equivalent to the public static nested class `LSTMBreakEngine.LSTMData`.
#[derive(Debug, Clone)]
pub struct LSTMData {
    /// `fType`.
    pub f_type: EmbeddingType,
    /// `fName`.
    pub f_name: Vec<u16>,
    /// `fDict`.
    pub f_dict: Arc<HashMap<Vec<u16>, i32>>,
    /// `fEmbedding`.
    pub f_embedding: Vec<Vec<f32>>,
    /// `fForwardW`.
    pub f_forward_w: Vec<Vec<f32>>,
    /// `fForwardU`.
    pub f_forward_u: Vec<Vec<f32>>,
    /// `fForwardB`.
    pub f_forward_b: Vec<f32>,
    /// `fBackwardW`.
    pub f_backward_w: Vec<Vec<f32>>,
    /// `fBackwardU`.
    pub f_backward_u: Vec<Vec<f32>>,
    /// `fBackwardB`.
    pub f_backward_b: Vec<f32>,
    /// `fOutputW`.
    pub f_output_w: Vec<Vec<f32>>,
    /// `fOutputB`.
    pub f_output_b: Vec<f32>,
}

impl LSTMData {
    /// `LSTMData(UResourceBundle rb)`: reads a model bundle.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` or `UResourceTypeMismatchException` for
    /// a malformed bundle.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` when `data` is shorter than the
    /// dimensions require.
    pub fn new(rb: &UResourceBundle) -> Result<Self> {
        let embeddings = rb.get("embeddings")?.get_int()?;
        let hunits = rb.get("hunits")?.get_int()?;
        let mut f_type = EmbeddingType::Unknown;
        let f_name = rb.get("model")?.get_string()?;
        let type_string = rb.get("type")?.get_string()?;
        if type_string == units("codepoints") {
            f_type = EmbeddingType::CodePoints;
        } else if type_string == units("graphclust") {
            f_type = EmbeddingType::GraphemeCluster;
        }
        let dict = rb.get("dict")?.get_string_array()?;
        let data = rb.get("data")?.get_int_vector()?;
        let data_len = data.len() as i32;
        let num_index = dict.len() as i32;
        let mut f_dict = HashMap::with_capacity((num_index + 1) as usize);
        for (idx, embedding) in dict.into_iter().enumerate() {
            f_dict.insert(embedding, idx as i32);
        }
        let mat1_size = (num_index + 1) * embeddings;
        let mat2_size = embeddings * 4 * hunits;
        let mat3_size = hunits * 4 * hunits;
        let mat4_size = 4 * hunits;
        let mat5_size = mat2_size;
        let mat6_size = mat3_size;
        let mat7_size = mat4_size;
        let mat8_size = 2 * hunits * 4;
        let mat9_size = 4;
        debug_assert!(
            data_len
                == mat1_size
                    + mat2_size
                    + mat3_size
                    + mat4_size
                    + mat5_size
                    + mat6_size
                    + mat7_size
                    + mat8_size
                    + mat9_size
        );
        let (e, h) = (embeddings as usize, hunits as usize);
        let mut start = 0usize;
        let f_embedding = make_2d_array(&data, start, (num_index + 1) as usize, e);
        start += mat1_size as usize;
        let f_forward_w = make_2d_array(&data, start, e, 4 * h);
        start += mat2_size as usize;
        let f_forward_u = make_2d_array(&data, start, h, 4 * h);
        start += mat3_size as usize;
        let f_forward_b = make_1d_array(&data, start, 4 * h);
        start += mat4_size as usize;
        let f_backward_w = make_2d_array(&data, start, e, 4 * h);
        start += mat5_size as usize;
        let f_backward_u = make_2d_array(&data, start, h, 4 * h);
        start += mat6_size as usize;
        let f_backward_b = make_1d_array(&data, start, 4 * h);
        start += mat7_size as usize;
        let f_output_w = make_2d_array(&data, start, 2 * h, 4);
        start += mat8_size as usize;
        let f_output_b = make_1d_array(&data, start, 4);
        Ok(Self {
            f_type,
            f_name,
            f_dict: Arc::new(f_dict),
            f_embedding,
            f_forward_w,
            f_forward_u,
            f_forward_b,
            f_backward_w,
            f_backward_u,
            f_backward_b,
            f_output_w,
            f_output_b,
        })
    }
}

/// `MIN_WORD`: minimum word size.
const MIN_WORD: i32 = 2;

/// `MIN_WORD_SPAN`: minimum number of characters for two words.
const MIN_WORD_SPAN: i32 = MIN_WORD * 2;

/// The conversion of a text range into model input indices.
///
/// Equivalent to the inner abstract class `LSTMBreakEngine.Vectorizer` and
/// its subclasses `CodePointsVectorizer` and `GraphemeClusterVectorizer`.
#[derive(Debug, Clone)]
enum Vectorizer {
    /// `CodePointsVectorizer`.
    CodePoints(Arc<HashMap<Vec<u16>, i32>>),
    /// `GraphemeClusterVectorizer`.
    GraphemeCluster(Arc<HashMap<Vec<u16>, i32>>),
}

impl Vectorizer {
    /// `Vectorizer.getIndex(String token)`: the token's index, or the
    /// dictionary size for an unknown token.
    fn get_index(dict: &HashMap<Vec<u16>, i32>, token: &[u16]) -> i32 {
        match dict.get(token) {
            None => dict.len() as i32,
            Some(&res) => res,
        }
    }

    /// `vectorize(CharacterIterator fIter, int rangeStart, int rangeEnd,
    /// List<Integer> offsets, List<Integer> indicies)`.
    fn vectorize(
        &self,
        text: &mut dyn CharacterIterator,
        start_pos: i32,
        end_pos: i32,
        offsets: &mut Vec<i32>,
        indicies: &mut Vec<i32>,
    ) {
        match self {
            Vectorizer::CodePoints(dict) => {
                set_index(text, start_pos);
                let mut c = text.current();
                while c != DONE && text.get_index() < end_pos {
                    offsets.push(text.get_index());
                    indicies.push(Self::get_index(dict, &[c]));
                    c = text.next();
                }
            }
            Vectorizer::GraphemeCluster(dict) => {
                // **Divergence.** Java gives the break iterator the caller's
                // CharacterIterator object itself (`LSTMBreakEngine.java`,
                // `iter.setText(text)`); a boxed break-iterator text cannot
                // borrow it, so the iterator walks a clone. The caller resets
                // its index after the range (`DictionaryBreakEngine.findBreaks`)
                // and `substring` restores it, so the outcome is the same.
                let mut iter =
                    break_iterator::get_character_instance().unwrap_or_else(|e| panic!("{e}"));
                iter.set_text(Some(text.clone_box()));
                let mut last = iter.next_n(start_pos);
                let mut curr = iter.next();
                while curr != BI_DONE && curr <= end_pos {
                    offsets.push(last);
                    let segment = Self::substring(text, last, curr);
                    let index = Self::get_index(dict, &segment);
                    indicies.push(index);
                    last = curr;
                    curr = iter.next();
                }
            }
        }
    }

    /// `GraphemeClusterVectorizer.substring(CharacterIterator text, int
    /// startPos, int endPos)`.
    fn substring(text: &mut dyn CharacterIterator, start_pos: i32, end_pos: i32) -> Vec<u16> {
        let saved = text.get_index();
        set_index(text, start_pos);
        let mut sb = Vec::new();
        let mut c = text.current();
        while c != DONE && text.get_index() < end_pos {
            sb.push(c);
            c = text.next();
        }
        set_index(text, saved);
        sb
    }
}

/// The LSTM-based engine for Thai and Burmese.
///
/// Equivalent to the class `com.ibm.icu.impl.breakiter.LSTMBreakEngine`.
#[derive(Debug)]
pub struct LSTMBreakEngine {
    base: DictionaryBreakEngineBase,
    f_data: LSTMData,
    f_script: i32,
    f_vectorizer: Option<Vectorizer>,
}

impl LSTMBreakEngine {
    /// `makeVectorizer(LSTMData data)`.
    fn make_vectorizer(data: &LSTMData) -> Option<Vectorizer> {
        match data.f_type {
            EmbeddingType::CodePoints => Some(Vectorizer::CodePoints(data.f_dict.clone())),
            EmbeddingType::GraphemeCluster => {
                Some(Vectorizer::GraphemeCluster(data.f_dict.clone()))
            }
            _ => None,
        }
    }

    /// `LSTMBreakEngine(int script, UnicodeSet set, LSTMData data)`.
    ///
    /// # Errors
    ///
    /// As `UnicodeSet.compact()`.
    pub fn new(script: i32, set: &UnicodeSet, data: LSTMData) -> Result<Self> {
        let mut base = DictionaryBreakEngineBase::new();
        base.set_characters(set)?;
        let f_vectorizer = Self::make_vectorizer(&data);
        Ok(Self {
            base,
            f_data: data,
            f_script: script,
            f_vectorizer,
        })
    }

    /// `hashCode()`: `getClass().hashCode()`.
    ///
    /// **Divergence.** Java answers the identity hash of the `Class` object
    /// (`LSTMBreakEngine.java`, `hashCode`), which no Rust value has; the
    /// port answers the crate's stable stand-in for it.
    pub fn hash_code(&self) -> i32 {
        java_identity_hash_code_stand_in("com.ibm.icu.impl.breakiter.LSTMBreakEngine")
    }

    /// `addDotProductTo(float[] a, float[][] b, float[] result)`.
    fn add_dot_product_to(a: &[f32], b: &[Vec<f32>], result: &mut [f32]) {
        debug_assert!(a.len() == b.len());
        debug_assert!(b[0].len() == result.len());
        for (i, r) in result.iter_mut().enumerate() {
            for (j, &aj) in a.iter().enumerate() {
                *r += aj * b[j][i];
            }
        }
    }

    /// `addTo(float[] a, float[] result)`.
    #[allow(dead_code)]
    fn add_to(a: &[f32], result: &mut [f32]) {
        debug_assert!(a.len() == result.len());
        for (r, &x) in result.iter_mut().zip(a) {
            *r += x;
        }
    }

    /// `hadamardProductTo(float[] a, float[] result)`.
    fn hadamard_product_to(a: &[f32], result: &mut [f32]) {
        debug_assert!(a.len() == result.len());
        for (r, &x) in result.iter_mut().zip(a) {
            *r *= x;
        }
    }

    /// `addHadamardProductTo(float[] a, float[] b, float[] result)`.
    fn add_hadamard_product_to(a: &[f32], b: &[f32], result: &mut [f32]) {
        debug_assert!(a.len() == result.len());
        debug_assert!(b.len() == result.len());
        for i in 0..result.len() {
            result[i] += a[i] * b[i];
        }
    }

    /// `sigmoid(float[] result, int start, int length)`.
    ///
    /// `Math.exp` is [`java_math::exp`] (fdlibm, the JDK 25 result).
    fn sigmoid(result: &mut [f32], start: usize, length: usize) {
        debug_assert!(start < result.len());
        debug_assert!(start + length <= result.len());
        for r in result.iter_mut().skip(start).take(length) {
            *r = (1.0 / (1.0 + java_math::exp(-(*r as f64)))) as f32;
        }
    }

    /// `tanh(float[] result, int start, int length)`.
    ///
    /// `Math.tanh` is [`java_math::tanh`] (fdlibm, the JDK 25 result).
    fn tanh(result: &mut [f32], start: usize, length: usize) {
        debug_assert!(start < result.len());
        debug_assert!(start + length <= result.len());
        for r in result.iter_mut().skip(start).take(length) {
            *r = java_math::tanh(*r as f64) as f32;
        }
    }

    /// `maxIndex(float[] data)`.
    fn max_index(data: &[f32]) -> i32 {
        let mut index = 0;
        let mut max = data[0];
        for (i, &d) in data.iter().enumerate().skip(1) {
            if d > max {
                max = d;
                index = i as i32;
            }
        }
        index
    }

    /// `compute(float[][] W, float[][] U, float[] B, float[] x, float[] h,
    /// float[] c)`: one LSTM step; updates `c` and returns the new `h`.
    fn compute(
        w: &[Vec<f32>],
        u: &[Vec<f32>],
        b: &[f32],
        x: &[f32],
        h: &[f32],
        c: &mut [f32],
    ) -> Vec<f32> {
        // ifco = x * W + h * U + b
        let mut ifco = b.to_vec();
        Self::add_dot_product_to(x, w, &mut ifco);
        let _h_u = vec![0f32; b.len()];
        Self::add_dot_product_to(h, u, &mut ifco);

        let hunits = b.len() / 4;
        Self::sigmoid(&mut ifco, 0, hunits); // i
        Self::sigmoid(&mut ifco, hunits, hunits); // f
        Self::tanh(&mut ifco, 2 * hunits, hunits); // c_
        Self::sigmoid(&mut ifco, 3 * hunits, hunits); // o

        Self::hadamard_product_to(&ifco[hunits..2 * hunits], c);
        Self::add_hadamard_product_to(&ifco[..hunits], &ifco[2 * hunits..3 * hunits], c);

        let mut h = c.to_vec();
        let len = h.len();
        Self::tanh(&mut h, 0, len);
        Self::hadamard_product_to(&ifco[3 * hunits..4 * hunits], &mut h);
        // System.out.println(Arrays.toString(h));
        h
    }

    /// `createData(UResourceBundle bundle)`.
    ///
    /// # Errors
    ///
    /// As [`LSTMData::new`].
    pub fn create_data(bundle: &UResourceBundle) -> Result<LSTMData> {
        LSTMData::new(bundle)
    }

    /// `defaultLSTM(int script)`: the model file the `brkitr` root bundle
    /// names for `script` under `lstm/`.
    fn default_lstm(script: i32) -> Result<Vec<u16>> {
        let rb = UResourceBundle::get_bundle_instance_default(Some(ICUData::ICU_BRKITR_BASE_NAME))?;
        let rb = rb.as_icu()?;
        let short = UScript::get_short_name(script)?;
        rb.get_string_with_fallback(&format!("lstm/{}", short.as_deref().unwrap_or("null")))
    }

    /// `createData(int script)`: the model of `script`; `None` for a script
    /// other than Khmer, Lao, Myanmar and Thai.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the model is not in the data (always
    /// so for ICU 77.1).
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` when the model name has no `.`.
    pub fn create_data_for_script(script: i32) -> Result<Option<LSTMData>> {
        if script != UScript::KHMER
            && script != UScript::LAO
            && script != UScript::MYANMAR
            && script != UScript::THAI
        {
            return Ok(None);
        }
        let name = Self::default_lstm(script)?;
        let dot = crate::jdk::index_of_unit(&name, b'.' as u16, 0);
        if dot < 0 {
            crate::jdk::string_index_out_of_bounds(dot);
        }
        let name = to_rust_string(&name[..dot as usize]);

        let rb = UResourceBundle::get_bundle_instance_loader(
            Some(ICUData::ICU_BRKITR_BASE_NAME),
            &name,
            icu_resource_bundle::icu_data_class_loader(),
        )?;
        Ok(Some(Self::create_data(&rb)?))
    }

    /// `create(int script, LSTMData data)`: an engine for the characters of
    /// `script` with line-break class SA.
    ///
    /// # Errors
    ///
    /// The exceptions of the `UnicodeSet` pattern.
    ///
    /// # Panics
    ///
    /// `NullPointerException` when `data` is `None` (Java dereferences the
    /// `null` that `createData(int)` answers for another script).
    pub fn create(script: i32, data: Option<LSTMData>) -> Result<LSTMBreakEngine> {
        let short = UScript::get_short_name(script)?;
        let set_expr = format!(
            "[[:{}:]&[:LineBreak=SA:]]",
            short.as_deref().unwrap_or("null")
        );
        let mut set = UnicodeSet::new();
        set.apply_pattern(&units(&set_expr))?;
        set.compact()?;
        let data = data.unwrap_or_else(|| panic!("java.lang.NullPointerException: data"));
        LSTMBreakEngine::new(script, &set, data)
    }
}

impl LanguageBreakEngine for LSTMBreakEngine {
    fn handles(&self, c: i32) -> bool {
        self.f_script == UCharacter::get_int_property_value(c, UProperty::SCRIPT)
    }

    fn find_breaks(
        &self,
        text: &mut dyn CharacterIterator,
        start_pos: i32,
        end_pos: i32,
        found_breaks: &mut DequeI,
        is_phrase_breaking: bool,
    ) -> i32 {
        dictionary_find_breaks(
            self,
            text,
            start_pos,
            end_pos,
            found_breaks,
            is_phrase_breaking,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl DictionaryBreakEngine for LSTMBreakEngine {
    fn base(&self) -> &DictionaryBreakEngineBase {
        &self.base
    }

    fn divide_up_dictionary_range(
        &self,
        f_iter: &mut dyn CharacterIterator,
        range_start: i32,
        range_end: i32,
        found_breaks: &mut DequeI,
        _is_phrase_breaking: bool,
    ) -> i32 {
        let begin_size = found_breaks.size();

        if (range_end - range_start) < MIN_WORD_SPAN {
            return 0; // Not enough characters for word
        }
        let cap = (range_end - range_start).max(0) as usize;
        let mut offsets: Vec<i32> = Vec::with_capacity(cap);
        let mut indicies: Vec<i32> = Vec::with_capacity(cap);

        self.f_vectorizer
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: fVectorizer"))
            .vectorize(f_iter, range_start, range_end, &mut offsets, &mut indicies);

        // To save the needed memory usage, the following is different from the
        // Python or ICU4X implementation. We first perform the Backward LSTM
        // and then merge the iteration of the forward LSTM and the output layer
        // together because we only need to remember the h[t-1] for Forward LSTM.
        let input_seq_length = indicies.len();
        let hunits = self.f_data.f_forward_u.len();
        let mut c = vec![0f32; hunits];

        // TODO: limit size of hBackward. If input_seq_length is too big, we may
        // run out of memory.
        // Backward LSTM
        let mut h_backward = vec![vec![0f32; hunits]; input_seq_length];
        for i in (0..input_seq_length).rev() {
            if i != input_seq_length - 1 {
                h_backward[i] = h_backward[i + 1].clone();
            }
            h_backward[i] = Self::compute(
                &self.f_data.f_backward_w,
                &self.f_data.f_backward_u,
                &self.f_data.f_backward_b,
                &self.f_data.f_embedding[indicies[i] as usize],
                &h_backward[i],
                &mut c,
            );
        }

        c = vec![0f32; hunits];
        let mut forward_h = vec![0f32; hunits];
        let mut both = vec![0f32; 2 * hunits];

        // The following iteration merge the forward LSTM and the output layer
        // together.
        for i in 0..input_seq_length {
            // Forward LSTM
            forward_h = Self::compute(
                &self.f_data.f_forward_w,
                &self.f_data.f_forward_u,
                &self.f_data.f_forward_b,
                &self.f_data.f_embedding[indicies[i] as usize],
                &forward_h,
                &mut c,
            );

            both[..hunits].copy_from_slice(&forward_h[..hunits]);
            both[hunits..2 * hunits].copy_from_slice(&h_backward[i][..hunits]);

            //System.out.println("Merged " + Arrays.toString(both));

            // Output layer
            // logp = fbRow * fOutputW + fOutputB
            let mut logp = self.f_data.f_output_b.clone();
            Self::add_dot_product_to(&both, &self.f_data.f_output_w, &mut logp);

            let current = Self::max_index(&logp);

            // BIES logic.
            if (current == LSTMClass::Begin.ordinal() || current == LSTMClass::Single.ordinal())
                && i != 0
            {
                found_breaks.push(offsets[i]);
            }
        }

        found_breaks.size() - begin_size
    }
}
