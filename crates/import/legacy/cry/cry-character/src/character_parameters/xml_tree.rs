use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};

use crate::{xml_cdata_content, xml_general_reference_content, xml_text_content};

use super::types::{
    CharacterLegacyParameterSource, CharacterParametersLegacyNodeSource,
    CharacterParametersParseError,
};

#[derive(Default)]
pub(super) struct XmlTreeParser {
    stack: Vec<CharacterParametersLegacyNodeSource>,
    root: Option<CharacterParametersLegacyNodeSource>,
    pending_comments: Vec<String>,
}

impl XmlTreeParser {
    pub(super) fn parse(
        xml: &str,
    ) -> Result<CharacterParametersLegacyNodeSource, CharacterParametersParseError> {
        let mut parser = Self::default();
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(false);

        loop {
            match reader.read_event()? {
                Event::Start(event) => parser.start_element(&reader, &event)?,
                Event::Empty(event) => parser.empty_element(&reader, &event)?,
                Event::End(event) => {
                    let name = String::from_utf8_lossy(event.name().as_ref()).into_owned();
                    parser.end_element(&name)?;
                }
                Event::Text(event) => {
                    let text = xml_text_content(&event)?.into_owned();
                    parser.text(text)?;
                }
                Event::CData(event) => {
                    let text = xml_cdata_content(&event)?.into_owned();
                    parser.text(text)?;
                }
                Event::Comment(event) => parser.comment(event.as_ref()),
                Event::GeneralRef(event) => {
                    parser.text(xml_general_reference_content(&event)?.into_owned())?;
                }
                Event::PI(_) | Event::Decl(_) | Event::DocType(_) => {}
                Event::Eof => break,
            }
        }

        if let Some(open) = parser.stack.last() {
            return Err(CharacterParametersParseError::UnclosedElement {
                element: open.name.clone(),
            });
        }

        parser
            .root
            .ok_or(CharacterParametersParseError::MissingRoot)
    }

    fn start_element(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<(), CharacterParametersParseError> {
        let node = self.node_from_event(reader, event)?;
        self.stack.push(node);
        Ok(())
    }

    fn empty_element(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<(), CharacterParametersParseError> {
        let node = self.node_from_event(reader, event)?;
        self.add_node(node)
    }

    fn end_element(&mut self, name: &str) -> Result<(), CharacterParametersParseError> {
        let node =
            self.stack
                .pop()
                .ok_or_else(|| CharacterParametersParseError::UnexpectedEnd {
                    element: name.to_string(),
                })?;
        if node.name != name {
            return Err(CharacterParametersParseError::MismatchedEnd {
                expected: node.name,
                found: name.to_string(),
            });
        }
        self.add_node(node)
    }

    fn text(&mut self, text: String) -> Result<(), CharacterParametersParseError> {
        if text.trim().is_empty() {
            return Ok(());
        }
        let Some(current) = self.stack.last_mut() else {
            return Err(CharacterParametersParseError::TextOutsideRoot { text });
        };
        current.text.push(text);
        Ok(())
    }

    fn comment(&mut self, bytes: &[u8]) {
        let comment = String::from_utf8_lossy(bytes).trim().to_string();
        if comment.is_empty() {
            return;
        }
        if let Some(current) = self.stack.last_mut() {
            current.comments.push(comment);
        } else {
            self.pending_comments.push(comment);
        }
    }

    fn node_from_event(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<CharacterParametersLegacyNodeSource, CharacterParametersParseError> {
        Ok(CharacterParametersLegacyNodeSource {
            name: element_name(event),
            parameters: attributes(reader, event)?,
            text: Vec::new(),
            comments: std::mem::take(&mut self.pending_comments),
            children: Vec::new(),
        })
    }

    fn add_node(
        &mut self,
        node: CharacterParametersLegacyNodeSource,
    ) -> Result<(), CharacterParametersParseError> {
        if let Some(parent) = self.stack.last_mut() {
            parent.children.push(node);
            Ok(())
        } else if self.root.is_none() {
            self.root = Some(node);
            Ok(())
        } else {
            Err(CharacterParametersParseError::ElementAfterRoot { element: node.name })
        }
    }
}

fn attributes(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<Vec<CharacterLegacyParameterSource>, CharacterParametersParseError> {
    let mut attributes = Vec::new();
    for attribute in event.attributes() {
        let attribute = attribute?;
        let name = String::from_utf8_lossy(attribute.key.as_ref()).into_owned();
        let value = attribute
            .decoded_and_normalized_value(quick_xml::XmlVersion::default(), reader.decoder())?
            .into_owned();
        attributes.push(CharacterLegacyParameterSource { name, value });
    }
    Ok(attributes)
}

fn element_name(event: &BytesStart<'_>) -> String {
    String::from_utf8_lossy(event.name().as_ref()).into_owned()
}
