use std::{borrow::Cow, collections::BTreeMap};

use crate::decode::{parse_bytes, Reference};

pub(crate) struct Key<'a> {
    slice: &'a [u8],
}

impl<'a> Key<'a> {
    fn parse(slice: &'a [u8]) -> Self {
        let mut index = 0;
        while index < slice.len() {
            match slice[index] {
                b'&' | b'=' => break,
                _ => index += 1,
            }
        }

        Self {
            slice: &slice[..index],
        }
    }

    fn len(&self) -> usize {
        self.slice.len()
    }

    fn decode_to<'s>(&self, scratch: &'s mut Vec<u8>) -> Reference<'a, 's, [u8]> {
        parse_bytes(self.slice, scratch)
    }
}

pub(crate) struct Value<'a>(&'a [u8]);

impl<'a> Value<'a> {
    fn parse(slice: &'a [u8]) -> Option<Self> {
        if *slice.get(0)? == b'&' {
            return None;
        }

        let mut index = 1;
        while index < slice.len() {
            match slice[index] {
                b'&' => break,
                _ => index += 1,
            }
        }

        Some(Self(&slice[1..index]))
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn decode_to<'s>(&self, scratch: &'s mut Vec<u8>) -> Reference<'a, 's, [u8]> {
        parse_bytes(self.0, scratch)
    }

    // pub fn decode(&self) -> Cow<'a, [u8]> {
    //     let mut scratch = Vec::new();
    //     self.decode_to(&mut scratch).into_cow()
    // }

    pub fn slice(&self) -> &'a [u8] {
        self.0
    }
}

pub struct Pair<'a>(Key<'a>, Option<Value<'a>>);

impl<'a> Pair<'a> {
    fn parse(slice: &'a [u8]) -> Self {
        let key = Key::parse(slice);
        let value = Value::parse(&slice[key.len()..]);

        Self(key, value)
    }

    fn len(&self) -> usize {
        match &self.1 {
            Some(v) => self.0.len() + v.len() + 2,
            None => self.0.len() + 1,
        }
    }
}

pub struct DuplicateQueryString<'a> {
    pairs: BTreeMap<Cow<'a, [u8]>, Vec<Pair<'a>>>,
}

impl<'a> DuplicateQueryString<'a> {
    pub fn parse(slice: &'a [u8]) -> Self {
        let mut pairs: BTreeMap<Cow<'a, [u8]>, Vec<Pair<'a>>> = BTreeMap::new();
        let mut scratch = Vec::new();

        let mut index = 0;

        while index < slice.len() {
            let pair = Pair::parse(&slice[index..]);
            index += pair.len();

            let decoded_key = pair.0.decode_to(&mut scratch);

            if let Some(values) = pairs.get_mut(decoded_key.as_ref()) {
                values.push(pair)
            } else {
                pairs.insert(decoded_key.into_cow(), vec![pair]);
            }
        }

        Self { pairs }
    }

    pub fn keys(&self) -> Vec<&Cow<'a, [u8]>> {
        self.pairs.keys().collect()
    }

    pub fn values(&self, key: &'a [u8]) -> Option<Vec<Option<Cow<'a, [u8]>>>> {
        let mut scratch = Vec::new();

        Some(
            self.pairs
                .get(key)?
                .iter()
                .map(|p| p.1.as_ref().map(|v| v.decode_to(&mut scratch).into_cow()))
                .collect(),
        )
    }

    pub fn value(&self, key: &'a [u8]) -> Option<Option<Cow<'a, [u8]>>> {
        let mut scratch = Vec::new();

        self.pairs
            .get(key)?
            .iter()
            .last()
            .map(|p| p.1.as_ref().map(|v| v.decode_to(&mut scratch).into_cow()))
    }

    pub fn raw_values(&self, key: &'a [u8]) -> Option<Vec<Option<&'a [u8]>>> {
        Some(
            self.pairs
                .get(key)?
                .iter()
                .map(|p| p.1.as_ref().map(|v| v.slice()))
                .collect(),
        )
    }

    pub fn raw_value(&self, key: &'a [u8]) -> Option<Option<&'a [u8]>> {
        self.pairs
            .get(key)?
            .iter()
            .last()
            .map(|p| p.1.as_ref().map(|v| v.slice()))
    }
}

#[cfg(feature = "serde")]
mod de {
    use crate::de::{
        Error,
        __implementors::{IntoSizedIterator, ParsedSlice, RawSlice},
    };

    use super::DuplicateQueryString;

    impl<'a> DuplicateQueryString<'a> {
        pub(crate) fn into_iter(
            self,
        ) -> impl Iterator<
            Item = (
                ParsedSlice<'a>,
                DuplicateValueIter<impl Iterator<Item = RawSlice<'a>>>,
            ),
        > {
            self.pairs.into_iter().map(|(key, pairs)| {
                (
                    ParsedSlice(key),
                    DuplicateValueIter(
                        pairs
                            .into_iter()
                            .map(|v| RawSlice(v.1.map(|v| v.slice()).unwrap_or_default())),
                    ),
                )
            })
        }
    }

    pub(crate) struct DuplicateValueIter<I>(I);

    impl<'a, I> IntoSizedIterator<'a> for DuplicateValueIter<I>
    where
        I: Iterator<Item = RawSlice<'a>>,
    {
        type SizedIterator = I;
        type UnSizedIterator = I;

        fn into_sized_iterator(self, size: usize) -> Result<I, Error> {
            if self.0.size_hint().0 == size {
                Ok(self.0)
            } else {
                Err(Error::Custom("()".to_string()))
            }
        }

        fn into_unsized_iterator(self) -> I {
            self.0
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::DuplicateQueryString;

    #[test]
    fn parse_pair() {
        let slice = b"key=value";

        let parser = DuplicateQueryString::parse(slice);

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

        let parser = DuplicateQueryString::parse(slice);

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

        let parser = DuplicateQueryString::parse(slice);

        assert_eq!(parser.values(b"foo"), Some(vec![None]));
        assert_eq!(
            parser.values(b"foobar"),
            Some(vec![Some("".as_bytes().into())])
        );
    }

    #[test]
    fn parse_multiple_values() {
        let slice = b"foo=bar&foo=baz&foo=foobar&foo&foo=";

        let parser = DuplicateQueryString::parse(slice);

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
}
