use std::{borrow::Cow, collections::BTreeMap};

use crate::decode::{parse_bytes, parse_char, Reference};

#[derive(Default, Clone, Copy)]
pub struct Key<'a>(&'a [u8], Option<&'a [u8]>);

impl<'a> Key<'a> {
    fn parse(slice: &'a [u8]) -> (Self, usize) {
        let mut index = 0;
        while index < slice.len() {
            match slice[index] {
                b'[' => {
                    let res = Key::parse_remains(&slice[..index], &slice[(index + 1)..]);
                    return (res.0, res.1 + index + 1);
                }
                b'%' => {
                    // Percent encoded opening bracket
                    if index + 2 < slice.len()
                        && parse_char(slice[index + 1], slice[index + 2]) == Some(b'[')
                    {
                        let res = Key::parse_remains(&slice[..index], &slice[(index + 3)..]);
                        return (res.0, res.1 + index + 3);
                    };
                    index += 1;
                }
                b'&' | b'=' => break,
                _ => index += 1,
            }
        }

        (Self(&slice[..index], None), index)
    }

    fn parse_remains(key: &'a [u8], slice: &'a [u8]) -> (Self, usize) {
        let mut index = 0;
        while index < slice.len() {
            match slice[index] {
                b'&' | b'=' => break,
                _ => index += 1,
            }
        }

        (Self(key, Some(&slice[..index])), index)
    }

    fn subkey(self) -> Option<Self> {
        let remains = self.1?;

        let mut key_end_index = 0;
        let mut index = 0;
        while index < remains.len() {
            match remains[index] {
                b']' => {
                    key_end_index = index;
                    break;
                }
                b'%' => {
                    // Percent encoded opening bracket
                    if index + 2 < remains.len()
                        && parse_char(remains[index + 1], remains[index + 2]) == Some(b']')
                    {
                        key_end_index = index;
                        index += 2;
                        break;
                    };
                    index += 1;
                }
                _ => index += 1,
            }
            key_end_index = index;
        }

        if index + 1 < remains.len() && remains[index + 1] == b'[' {
            Some(Self(&remains[..key_end_index], Some(&remains[index + 2..])))
        } else if index + 3 < remains.len()
            && remains[index + 1] == b'%'
            && parse_char(remains[index + 2], remains[index + 3]) == Some(b'[')
        {
            Some(Self(&remains[..key_end_index], Some(&remains[index + 4..])))
        } else {
            Some(Self(&remains[..key_end_index], None))
        }
    }

    fn has_subkey(&self) -> bool {
        match self.1 {
            Some(remains) => {
                let mut index = 0;
                while index < remains.len() {
                    match remains[index] {
                        b']' => return true,
                        b'%' => {
                            // Percent encoded opening bracket
                            if index + 2 < remains.len()
                                && parse_char(remains[index + 1], remains[index + 2]) == Some(b']')
                            {
                                return true;
                            };
                            index += 1;
                        }
                        _ => index += 1,
                    }
                }
                return false;
            }
            None => false,
        }
    }

    fn is_empty(&self) -> bool {
        match self.1 {
            Some(r) => self.0.is_empty() && r.is_empty(),
            None => self.0.is_empty(),
        }
    }

    fn decode_to<'s>(&self, scratch: &'s mut Vec<u8>) -> Reference<'a, 's, [u8]> {
        parse_bytes(self.0, scratch)
    }
}

#[derive(Default, Clone, Copy)]
pub struct Value<'a>(&'a [u8]);

impl<'a> Value<'a> {
    fn parse(slice: &'a [u8]) -> (Option<Self>, usize) {
        match slice.get(0) {
            Some(b'&') | None => {
                return (None, 0);
            }
            _ => {}
        }

        let mut index = 1;
        while index < slice.len() {
            match slice[index] {
                b'&' => break,
                _ => index += 1,
            }
        }

        (Some(Self(&slice[1..index])), index)
    }

    fn decode_to<'s>(&self, scratch: &'s mut Vec<u8>) -> Reference<'a, 's, [u8]> {
        parse_bytes(self.0, scratch)
    }

    pub fn slice(&self) -> &'a [u8] {
        self.0
    }
}

#[derive(Default, Clone, Copy)]
pub struct Pair<'a>(Key<'a>, Option<Value<'a>>);

impl<'a> Pair<'a> {
    fn parse(slice: &'a [u8]) -> (Self, usize) {
        let (key, key_len) = Key::parse(slice);
        let (value, value_len) = Value::parse(&slice[key_len..]);

        (Self(key, value), key_len + value_len + 1)
    }

    fn new(k: Key<'a>, v: Option<Value<'a>>) -> Pair<'a> {
        Self(k, v)
    }
}

pub struct BracketsQS<'a> {
    pairs: BTreeMap<Cow<'a, [u8]>, Vec<Pair<'a>>>,
}

impl<'a> BracketsQS<'a> {
    pub fn parse(slice: &'a [u8]) -> Self {
        let mut pairs: BTreeMap<Cow<'a, [u8]>, Vec<Pair<'a>>> = BTreeMap::new();
        let mut scratch = Vec::new();

        let mut index = 0;

        while index < slice.len() {
            let (pair, pair_len) = Pair::parse(&slice[index..]);
            index += pair_len;

            let decoded_key = pair.0.decode_to(&mut scratch);

            if let Some(values) = pairs.get_mut(decoded_key.as_ref()) {
                values.push(pair);
            } else {
                pairs.insert(decoded_key.into_cow(), vec![pair]);
            }
        }

        Self { pairs }
    }

    pub fn from_pairs<I>(iter: I) -> Self
    where
        I: Iterator<Item = Pair<'a>>,
    {
        let mut pairs: BTreeMap<Cow<'a, [u8]>, Vec<Pair<'a>>> = BTreeMap::new();

        let mut scratch = Vec::new();
        let subpairs = iter.filter_map(|p| Some((p.0.subkey()?, p.1)));

        for (k, v) in subpairs {
            let decoded_key = k.decode_to(&mut scratch);
            let pair = Pair::new(k, v);

            if let Some(values) = pairs.get_mut(decoded_key.as_ref()) {
                values.push(pair);
            } else {
                pairs.insert(decoded_key.into_cow(), vec![pair]);
            }
        }

        Self { pairs }
    }

    pub fn keys(&self) -> Vec<&Cow<'a, [u8]>> {
        self.pairs.keys().collect()
    }

    pub fn sub_values(&self, key: &'a [u8]) -> Option<BracketsQS> {
        Some(Self::from_pairs(self.pairs.get(key)?.iter().copied()))
    }

    pub fn values(&self, key: &'a [u8]) -> Option<Vec<Option<Cow<'a, [u8]>>>> {
        let mut scratch = Vec::new();

        Some(
            self.pairs
                .get(key)?
                .iter()
                .filter(|p| !p.0.has_subkey())
                .map(|p| p.1.as_ref().map(|v| v.decode_to(&mut scratch).into_cow()))
                .collect(),
        )
    }

    pub fn value(&self, key: &'a [u8]) -> Option<Option<Cow<'a, [u8]>>> {
        let mut scratch = Vec::new();

        self.pairs
            .get(key)?
            .iter()
            .filter(|p| !p.0.has_subkey())
            .last()
            .map(|p| p.1.as_ref().map(|v| v.decode_to(&mut scratch).into_cow()))
    }
}

#[cfg(feature = "serde")]
mod de {
    use _serde::{de, forward_to_deserialize_any, Deserializer};

    use crate::de::{
        Error, ErrorKind,
        __implementors::{IntoDeserializer, ParsedSlice, RawSlice},
    };

    use super::{BracketsQS, Pair};

    pub struct Pairs<'a>(Vec<Pair<'a>>);

    impl<'a> BracketsQS<'a> {
        pub(crate) fn into_iter(self) -> impl Iterator<Item = (ParsedSlice<'a>, Pairs<'a>)> {
            self.pairs
                .into_iter()
                .map(|(key, pairs)| (ParsedSlice(key), Pairs(pairs)))
        }
    }

    impl<'a, 's> IntoDeserializer<'a, 's> for Pairs<'a> {
        type Deserializer = PairsDeserializer<'a, 's>;

        fn into_deserializer(self, scratch: &'s mut Vec<u8>) -> Self::Deserializer {
            PairsDeserializer(self.0, scratch)
        }
    }

    pub struct PairsDeserializer<'a, 's>(Vec<Pair<'a>>, &'s mut Vec<u8>);

    impl<'a, 's> PairsDeserializer<'a, 's> {
        #[inline]
        fn to_seq_values(&mut self) -> Result<Vec<(usize, RawSlice<'a>)>, Error> {
            let mut values = std::mem::take(&mut self.0)
                .into_iter()
                .map(|pair| {
                    let index = match pair.0.subkey() {
                        Some(subkey) if !subkey.is_empty() => lexical::parse::<usize, _>(subkey.0)
                            .map_err(|e| {
                                Error::new(ErrorKind::InvalidNumber)
                                    .message(format!("invalid index: {}", e))
                            })?,
                        _ => 0,
                    };
                    Ok((index, RawSlice(pair.1.unwrap_or_default().slice())))
                })
                .collect::<Result<Vec<(usize, RawSlice)>, Error>>()?;

            values.sort_by_key(|item| item.0);
            Ok(values)
        }
    }

    macro_rules! forware_to_slice_deserializer {
        ($($method:ident ,)*) => {
            $(
                #[inline]
                fn $method<V>(self, visitor: V) -> Result<V::Value, Error>
                where
                    V: de::Visitor<'de>,
                {
                    let scratch = self.1;
                    let value = self.0.last().unwrap().1.unwrap_or_default().slice();
                    RawSlice(value).into_deserializer(scratch).$method(visitor)
                }
            )*
        };
    }

    impl<'de, 's> de::Deserializer<'de> for PairsDeserializer<'de, 's> {
        type Error = crate::de::Error;

        fn deserialize_seq<V>(mut self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: de::Visitor<'de>,
        {
            visitor.visit_seq(PairsSeqDeserializer(
                self.to_seq_values()?.into_iter().map(|v| v.1),
                self.1,
            ))
        }

        fn deserialize_tuple<V>(mut self, len: usize, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: de::Visitor<'de>,
        {
            let values = self.to_seq_values()?;

            if values.len() == len {
                visitor.visit_seq(PairsSeqDeserializer(
                    values.into_iter().map(|v| v.1),
                    self.1,
                ))
            } else {
                Err(Error::new(ErrorKind::InvalidLength))
            }
        }

        fn deserialize_tuple_struct<V>(
            self,
            _: &'static str,
            len: usize,
            visitor: V,
        ) -> Result<V::Value, Self::Error>
        where
            V: de::Visitor<'de>,
        {
            self.deserialize_tuple(len, visitor)
        }

        fn deserialize_newtype_struct<V>(
            self,
            _: &'static str,
            visitor: V,
        ) -> Result<V::Value, Self::Error>
        where
            V: de::Visitor<'de>,
        {
            visitor.visit_newtype_struct(self)
        }

        fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: de::Visitor<'de>,
        {
            visitor.visit_map(PairsMapDeserializer {
                iter: BracketsQS::from_pairs(self.0.into_iter()).into_iter(),
                scratch: self.1,
                value: None,
            })
        }

        fn deserialize_struct<V>(
            self,
            _: &'static str,
            _: &'static [&'static str],
            visitor: V,
        ) -> Result<V::Value, Self::Error>
        where
            V: de::Visitor<'de>,
        {
            self.deserialize_map(visitor)
        }

        fn deserialize_enum<V>(
            self,
            _: &'static str,
            _: &'static [&'static str],
            visitor: V,
        ) -> Result<V::Value, Self::Error>
        where
            V: de::Visitor<'de>,
        {
            visitor.visit_enum(self)
        }

        forware_to_slice_deserializer! {
            deserialize_i8, deserialize_i16, deserialize_i32, deserialize_i64, deserialize_i128,
            deserialize_u8, deserialize_u16, deserialize_u32, deserialize_u64, deserialize_u128,
            deserialize_f32, deserialize_f64,
            deserialize_char, deserialize_str, deserialize_string, deserialize_identifier,
            deserialize_bool, deserialize_bytes, deserialize_byte_buf, deserialize_option, deserialize_unit,
            deserialize_any, deserialize_ignored_any,
        }

        forward_to_deserialize_any! {
            unit_struct
        }
    }

    impl<'de, 's> de::EnumAccess<'de> for PairsDeserializer<'de, 's> {
        type Error = Error;

        type Variant = Self;

        fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant), Self::Error>
        where
            V: de::DeserializeSeed<'de>,
        {
            let last_pair = self.0.last().expect("Values iterator can't be empty");
            match last_pair.0.subkey() {
                Some(subkey) => {
                    let mut scratch = self.1;
                    let pairs = BracketsQS::from_pairs(self.0.into_iter())
                        .pairs
                        .remove(subkey.0)
                        .unwrap();
                    seed.deserialize(RawSlice(subkey.0).into_deserializer(&mut scratch))
                        .map(move |v| (v, Self(pairs, scratch)))
                }
                None => {
                    let mut scratch = self.1;
                    seed.deserialize(
                        RawSlice(last_pair.1.unwrap_or_default().0).into_deserializer(&mut scratch),
                    )
                    .map(move |v| (v, PairsDeserializer(Vec::new(), scratch)))
                }
            }
        }
    }

    impl<'de, 's> de::VariantAccess<'de> for PairsDeserializer<'de, 's> {
        type Error = Error;

        fn unit_variant(self) -> Result<(), Self::Error> {
            if self.0.len() == 0 {
                Ok(())
            } else {
                Err(Error::new(ErrorKind::Other)
                    .message("Unit enum variants should not have values".to_string()))
            }
        }

        fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value, Self::Error>
        where
            T: de::DeserializeSeed<'de>,
        {
            seed.deserialize(self)
        }

        fn tuple_variant<V>(self, len: usize, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: de::Visitor<'de>,
        {
            self.deserialize_tuple(len, visitor)
        }

        fn struct_variant<V>(
            self,
            fields: &'static [&'static str],
            visitor: V,
        ) -> Result<V::Value, Self::Error>
        where
            V: de::Visitor<'de>,
        {
            self.deserialize_struct("name", fields, visitor)
        }
    }

    struct PairsSeqDeserializer<'s, I>(I, &'s mut Vec<u8>);

    impl<'de, 's, I> de::SeqAccess<'de> for PairsSeqDeserializer<'s, I>
    where
        I: Iterator<Item = RawSlice<'de>>,
    {
        type Error = Error;

        fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
        where
            T: de::DeserializeSeed<'de>,
        {
            if let Some(v) = self.0.next() {
                seed.deserialize(v.into_deserializer(self.1)).map(Some)
            } else {
                Ok(None)
            }
        }
    }

    struct PairsMapDeserializer<'de, 's, I>
    where
        I: Iterator<Item = (ParsedSlice<'de>, Pairs<'de>)>,
    {
        iter: I,
        scratch: &'s mut Vec<u8>,
        value: Option<Pairs<'de>>,
    }

    impl<'de, 's, I> de::MapAccess<'de> for PairsMapDeserializer<'de, 's, I>
    where
        I: Iterator<Item = (ParsedSlice<'de>, Pairs<'de>)>,
    {
        type Error = Error;

        fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
        where
            K: de::DeserializeSeed<'de>,
        {
            if let Some((k, v)) = self.iter.next() {
                self.value = Some(v);

                seed.deserialize(k.into_deserializer(self.scratch))
                    .map(Some)
            } else {
                Ok(None)
            }
        }

        fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
        where
            V: de::DeserializeSeed<'de>,
        {
            seed.deserialize(
                self.value
                    .take()
                    .expect("next_value is called before next_key")
                    .into_deserializer(self.scratch),
            )
        }

        fn size_hint(&self) -> Option<usize> {
            self.iter.size_hint().1
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::BracketsQS;

    #[test]
    fn parse_pair() {
        let slice = b"key=value";

        let parser = BracketsQS::parse(slice);

        assert_eq!(parser.keys(), vec![&Cow::Borrowed(b"key")]);
        assert_eq!(
            parser.values(b"key"),
            Some(vec![Some(Cow::Borrowed("value".as_bytes()))])
        );
        assert_eq!(
            parser.value(b"key"),
            Some(Some(Cow::Borrowed("value".as_bytes())))
        );

        assert_eq!(parser.values(b"test"), None);
    }

    #[test]
    fn parse_multiple_pairs() {
        let slice = b"foo=bar&foobar=baz&qux=box";

        let parser = BracketsQS::parse(slice);

        assert_eq!(
            parser.values(b"foo"),
            Some(vec![Some("bar".as_bytes().into())])
        );
        assert_eq!(
            parser.values(b"foobar"),
            Some(vec![Some("baz".as_bytes().into())])
        );
        assert_eq!(
            parser.values(b"qux"),
            Some(vec![Some("box".as_bytes().into())])
        );
    }

    #[test]
    fn parse_no_value() {
        let slice = b"foo&foobar=";

        let parser = BracketsQS::parse(slice);

        assert_eq!(parser.values(b"foo"), Some(vec![None]));
        assert_eq!(
            parser.values(b"foobar"),
            Some(vec![Some("".as_bytes().into())])
        );
    }

    #[test]
    fn parse_multiple_values() {
        let slice = b"foo=bar&foo=baz&foo=foobar&foo&foo=";

        let parser = BracketsQS::parse(slice);

        assert_eq!(
            parser.values(b"foo"),
            Some(vec![
                Some("bar".as_bytes().into()),
                Some("baz".as_bytes().into()),
                Some("foobar".as_bytes().into()),
                None,
                Some("".as_bytes().into())
            ])
        );

        assert_eq!(parser.value(b"foo"), Some(Some("".as_bytes().into())));
    }

    #[test]
    fn parse_subkeys() {
        let slice = b"foo[bar]=baz&foo[bar]=buzz&foo[foobar]=qux&foo=bar";

        let parser = BracketsQS::parse(slice);

        assert_eq!(
            parser.values(b"foo"),
            Some(vec![Some("bar".as_bytes().into())])
        );

        let foo_values = parser.sub_values(b"foo");
        assert!(foo_values.is_some());

        let foo_values = foo_values.unwrap();

        assert_eq!(
            foo_values.values(b"bar"),
            Some(vec![
                Some("baz".as_bytes().into()),
                Some("buzz".as_bytes().into())
            ])
        );

        assert_eq!(
            foo_values.values(b"foobar"),
            Some(vec![Some("qux".as_bytes().into())])
        )
    }

    #[test]
    fn parse_invalid() {
        // Invalid suffix of keys should be ignored

        let slice = b"foo[bar]xyz=baz&foo[bar][xyz=buzz&foo[foobar]xyz]=qux&foo[xyz=bar";

        let parser = BracketsQS::parse(slice);

        assert_eq!(
            parser.values(b"foo"),
            Some(vec![Some("bar".as_bytes().into())])
        );

        let foo_values = parser.sub_values(b"foo");
        assert!(foo_values.is_some());

        let foo_values = foo_values.unwrap();

        assert_eq!(
            foo_values.values(b"bar"),
            Some(vec![
                Some("baz".as_bytes().into()),
                Some("buzz".as_bytes().into())
            ])
        );

        assert_eq!(
            foo_values.values(b"foobar"),
            Some(vec![Some("qux".as_bytes().into())])
        )
    }
}
