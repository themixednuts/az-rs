use std::collections::{BTreeMap, BTreeSet};

use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};

use crate::{xml_cdata_content, xml_general_reference_content, xml_text_content};

use super::{
    MaterialEffectReferenceSource, MaterialEffectsInteractionAxisEntrySource,
    MaterialEffectsInteractionCellSource, MaterialEffectsInteractionIndexSource,
    MaterialEffectsInteractionRowKindSource, MaterialEffectsInteractionRowSource,
    MaterialEffectsParseError, MaterialEffectsSource, MaterialEffectsSpreadsheetCellMetadataSource,
    str,
};

const WORKSHEET_NAME: &str = "MFX";

pub(super) fn parse_interaction_index(
    source_path: String,
    xml: &str,
) -> Result<MaterialEffectsSource, MaterialEffectsParseError> {
    MaterialEffectsSpreadsheetParser::new(source_path).parse(xml)
}

struct MaterialEffectsSpreadsheetParser {
    source_path: String,
    saw_target_worksheet: bool,
    in_target_worksheet: bool,
    in_table: bool,
    row_sequence: u32,
    current_row: Option<SpreadsheetRowBuilder>,
    current_cell: Option<SpreadsheetCellBuilder>,
    rows: Vec<SpreadsheetRow>,
    comments: Vec<String>,
}

impl MaterialEffectsSpreadsheetParser {
    const fn new(source_path: String) -> Self {
        Self {
            source_path,
            saw_target_worksheet: false,
            in_target_worksheet: false,
            in_table: false,
            row_sequence: 0,
            current_row: None,
            current_cell: None,
            rows: Vec::new(),
            comments: Vec::new(),
        }
    }

    fn parse(mut self, xml: &str) -> Result<MaterialEffectsSource, MaterialEffectsParseError> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(false);

        loop {
            match reader.read_event()? {
                Event::Start(event) => self.start_element(&reader, &event)?,
                Event::Empty(event) => {
                    self.start_element(&reader, &event)?;
                    self.end_element(&element_name(&event))?;
                }
                Event::End(event) => {
                    let name = local_name(event.name().as_ref());
                    self.end_element(&name)?;
                }
                Event::Text(event) => {
                    let text = xml_text_content(&event)?.into_owned();
                    self.text(text)?;
                }
                Event::CData(event) => {
                    let text = xml_cdata_content(&event)?.into_owned();
                    self.text(text)?;
                }
                Event::Comment(event) => self.comment(event.as_ref()),
                Event::GeneralRef(event) => {
                    self.text(xml_general_reference_content(&event)?.into_owned())?;
                }
                Event::PI(_) | Event::Decl(_) | Event::DocType(_) => {}
                Event::Eof => break,
            }
        }

        if self.current_cell.is_some() {
            return Err(MaterialEffectsParseError::UnclosedElement {
                element: "Cell".to_string(),
            });
        }
        if self.current_row.is_some() {
            return Err(MaterialEffectsParseError::UnclosedElement {
                element: "Row".to_string(),
            });
        }
        if self.in_table {
            return Err(MaterialEffectsParseError::UnclosedElement {
                element: "Table".to_string(),
            });
        }
        if self.in_target_worksheet {
            return Err(MaterialEffectsParseError::UnclosedElement {
                element: "Worksheet".to_string(),
            });
        }
        if !self.saw_target_worksheet {
            return Err(MaterialEffectsParseError::MissingWorksheet {
                name: WORKSHEET_NAME,
            });
        }

        self.finish()
    }

    fn start_element(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<(), MaterialEffectsParseError> {
        let name = element_name(event);
        match name.as_str() {
            "Worksheet" => self.start_worksheet(reader, event),
            "Table" if self.in_target_worksheet => {
                self.in_table = true;
                Ok(())
            }
            "Row" if self.in_table => self.start_row(reader, event),
            "Cell" if self.in_table => self.start_cell(reader, event),
            "Data" if self.current_cell.is_some() => self.start_data(reader, event),
            _ => Ok(()),
        }
    }

    fn end_element(&mut self, name: &str) -> Result<(), MaterialEffectsParseError> {
        match name {
            "Worksheet" if self.in_target_worksheet => {
                if self.current_row.is_some() {
                    return Err(MaterialEffectsParseError::UnclosedElement {
                        element: "Row".to_string(),
                    });
                }
                self.in_target_worksheet = false;
                Ok(())
            }
            "Table" if self.in_target_worksheet && self.in_table => {
                if self.current_row.is_some() {
                    return Err(MaterialEffectsParseError::UnclosedElement {
                        element: "Row".to_string(),
                    });
                }
                self.in_table = false;
                Ok(())
            }
            "Row" if self.current_row.is_some() => self.finish_row(),
            "Cell" if self.current_cell.is_some() => self.finish_cell(),
            "Data" if self.current_cell.is_some() => self.finish_data(),
            _ => Ok(()),
        }
    }

    fn start_worksheet(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<(), MaterialEffectsParseError> {
        let worksheet_name = attribute_value(reader, event, "Name")?;
        if worksheet_name.as_deref() != Some(WORKSHEET_NAME) {
            return Ok(());
        }

        if self.saw_target_worksheet {
            return Err(MaterialEffectsParseError::DuplicateElement {
                element: "Worksheet",
            });
        }

        self.saw_target_worksheet = true;
        self.in_target_worksheet = true;
        Ok(())
    }

    fn start_row(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<(), MaterialEffectsParseError> {
        if self.current_row.is_some() {
            return Err(MaterialEffectsParseError::NestedElement { element: "Row" });
        }
        if self.current_cell.is_some() {
            return Err(MaterialEffectsParseError::ElementInWrongParent {
                element: "Row",
                parent: "Table",
            });
        }

        let index = indexed_sequence_value(reader, event, "Row", self.row_sequence + 1)?;
        self.row_sequence = index;
        self.current_row = Some(SpreadsheetRowBuilder {
            index,
            next_cell_index: 1,
            cells: Vec::new(),
        });
        Ok(())
    }

    fn start_cell(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<(), MaterialEffectsParseError> {
        if self.current_cell.is_some() {
            return Err(MaterialEffectsParseError::NestedElement { element: "Cell" });
        }

        let row =
            self.current_row
                .as_mut()
                .ok_or(MaterialEffectsParseError::ElementInWrongParent {
                    element: "Cell",
                    parent: "Row",
                })?;
        let attributes = attributes(reader, event)?;
        let mut formula = None;
        let mut index = row.next_cell_index;

        for attribute in attributes {
            match attribute.name.as_str() {
                "Formula" => formula = Some(attribute.value),
                "Index" => {
                    index = parse_u32("Cell", "Index", &attribute.value)?;
                }
                _ => {}
            }
        }

        row.next_cell_index = index.saturating_add(1);
        self.current_cell = Some(SpreadsheetCellBuilder {
            index,
            text: String::new(),
            metadata: MaterialEffectsSpreadsheetCellMetadataSource {
                formula,
                value_type: None,
                rel_version: None,
            },
            data_open: false,
        });
        Ok(())
    }

    fn start_data(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<(), MaterialEffectsParseError> {
        let attributes = attributes(reader, event)?;
        let cell =
            self.current_cell
                .as_mut()
                .ok_or(MaterialEffectsParseError::ElementInWrongParent {
                    element: "Data",
                    parent: "Cell",
                })?;
        if cell.data_open {
            return Err(MaterialEffectsParseError::NestedTextElement { element: "Data" });
        }

        for attribute in attributes {
            match attribute.name.as_str() {
                "Type" => cell.metadata.value_type = Some(attribute.value),
                "rel_version" => cell.metadata.rel_version = Some(attribute.value),
                _ => {}
            }
        }

        cell.data_open = true;
        Ok(())
    }

    fn finish_data(&mut self) -> Result<(), MaterialEffectsParseError> {
        let cell = self.current_cell.as_mut().ok_or_else(|| {
            MaterialEffectsParseError::UnexpectedEndElement {
                element: "Data".to_string(),
            }
        })?;
        if !cell.data_open {
            return Err(MaterialEffectsParseError::UnexpectedEndElement {
                element: "Data".to_string(),
            });
        }

        cell.data_open = false;
        Ok(())
    }

    fn finish_cell(&mut self) -> Result<(), MaterialEffectsParseError> {
        let cell = self.current_cell.take().ok_or_else(|| {
            MaterialEffectsParseError::UnexpectedEndElement {
                element: "Cell".to_string(),
            }
        })?;
        if cell.data_open {
            return Err(MaterialEffectsParseError::UnclosedElement {
                element: "Data".to_string(),
            });
        }

        let row =
            self.current_row
                .as_mut()
                .ok_or(MaterialEffectsParseError::ElementInWrongParent {
                    element: "Cell",
                    parent: "Row",
                })?;
        row.cells.push(cell.finish());
        Ok(())
    }

    fn finish_row(&mut self) -> Result<(), MaterialEffectsParseError> {
        if self.current_cell.is_some() {
            return Err(MaterialEffectsParseError::UnclosedElement {
                element: "Cell".to_string(),
            });
        }

        let row = self
            .current_row
            .take()
            .ok_or_else(|| MaterialEffectsParseError::UnexpectedEndElement {
                element: "Row".to_string(),
            })?
            .finish();
        if row.has_semantic_cells() {
            self.rows.push(row);
        }
        Ok(())
    }

    fn text(&mut self, text: String) -> Result<(), MaterialEffectsParseError> {
        if let Some(cell) = self.current_cell.as_mut()
            && cell.data_open
        {
            cell.text.push_str(&text);
            return Ok(());
        }

        if self.in_table && !text.trim().is_empty() {
            return Err(MaterialEffectsParseError::UnexpectedText { text });
        }

        Ok(())
    }

    fn comment(&mut self, bytes: &[u8]) {
        if !self.in_target_worksheet {
            return;
        }

        let comment = String::from_utf8_lossy(bytes).trim().to_string();
        if !comment.is_empty() {
            self.comments.push(comment);
        }
    }

    fn finish(self) -> Result<MaterialEffectsSource, MaterialEffectsParseError> {
        let mut rows = self.rows.into_iter();
        let Some(header) = rows.next() else {
            return Err(MaterialEffectsParseError::MissingHeaderRow);
        };

        let columns: Vec<_> = header
            .cells
            .into_iter()
            .filter(|cell| cell.index > 1 && cell.has_text())
            .map(|cell| MaterialEffectsInteractionAxisEntrySource {
                index: cell.index,
                name: cell.text,
                metadata: cell.metadata,
            })
            .collect();
        if columns.is_empty() {
            return Err(MaterialEffectsParseError::MissingHeaderRow);
        }

        let columns_by_index: BTreeMap<_, _> = columns
            .iter()
            .map(|column| (column.index, column.name.clone()))
            .collect();
        let surface_names: BTreeSet<_> = columns
            .iter()
            .map(|column| column.name.to_ascii_lowercase())
            .collect();

        let mut output_rows = Vec::new();
        for row in rows {
            output_rows.push(finish_interaction_row(
                row,
                &columns_by_index,
                &surface_names,
            )?);
        }

        Ok(MaterialEffectsSource::InteractionIndex(
            MaterialEffectsInteractionIndexSource {
                source_path: self.source_path,
                worksheet: WORKSHEET_NAME.to_string(),
                columns,
                rows: output_rows,
                comments: self.comments,
            },
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct XmlAttribute {
    name: String,
    value: String,
}

#[derive(Debug)]
struct SpreadsheetRowBuilder {
    index: u32,
    next_cell_index: u32,
    cells: Vec<SpreadsheetCell>,
}

impl SpreadsheetRowBuilder {
    fn finish(self) -> SpreadsheetRow {
        SpreadsheetRow {
            index: self.index,
            cells: self.cells,
        }
    }
}

#[derive(Debug)]
struct SpreadsheetCellBuilder {
    index: u32,
    text: String,
    metadata: MaterialEffectsSpreadsheetCellMetadataSource,
    data_open: bool,
}

impl SpreadsheetCellBuilder {
    fn finish(mut self) -> SpreadsheetCell {
        self.text = self.text.trim().to_string();
        SpreadsheetCell {
            index: self.index,
            text: self.text,
            metadata: self.metadata,
        }
    }
}

#[derive(Debug)]
struct SpreadsheetRow {
    index: u32,
    cells: Vec<SpreadsheetCell>,
}

impl SpreadsheetRow {
    fn has_semantic_cells(&self) -> bool {
        self.cells.iter().any(SpreadsheetCell::has_text)
    }
}

#[derive(Debug, Clone)]
struct SpreadsheetCell {
    index: u32,
    text: String,
    metadata: MaterialEffectsSpreadsheetCellMetadataSource,
}

impl SpreadsheetCell {
    const fn has_text(&self) -> bool {
        !self.text.is_empty()
    }
}

fn finish_interaction_row(
    row: SpreadsheetRow,
    columns_by_index: &BTreeMap<u32, String>,
    surface_names: &BTreeSet<String>,
) -> Result<MaterialEffectsInteractionRowSource, MaterialEffectsParseError> {
    let label_cell = row
        .cells
        .iter()
        .find(|cell| cell.index == 1 && cell.has_text())
        .cloned();
    let Some(label_cell) = label_cell else {
        return Err(MaterialEffectsParseError::MissingRowLabel {
            row_index: row.index,
        });
    };

    let mut entries = Vec::new();
    for cell in row
        .cells
        .into_iter()
        .filter(|cell| cell.index > 1 && cell.has_text())
    {
        let Some(column) = columns_by_index.get(&cell.index) else {
            return Err(MaterialEffectsParseError::EntryWithoutColumn {
                row_index: row.index,
                column_index: cell.index,
            });
        };

        entries.push(MaterialEffectsInteractionCellSource {
            column_index: cell.index,
            column: column.clone(),
            reference: parse_reference(cell.text),
            metadata: cell.metadata,
        });
    }

    let kind = if surface_names.contains(&label_cell.text.to_ascii_lowercase()) {
        MaterialEffectsInteractionRowKindSource::Surface
    } else {
        MaterialEffectsInteractionRowKindSource::Custom
    };

    Ok(MaterialEffectsInteractionRowSource {
        index: row.index,
        kind,
        name: label_cell.text,
        metadata: label_cell.metadata,
        entries,
    })
}

fn parse_reference(raw: String) -> MaterialEffectReferenceSource {
    if let Some((library, effect)) = raw.split_once(':') {
        let library = library.to_string();
        let effect = effect.to_string();
        MaterialEffectReferenceSource {
            raw,
            library,
            effect,
        }
    } else {
        MaterialEffectReferenceSource {
            library: raw.clone(),
            effect: raw.clone(),
            raw,
        }
    }
}

fn indexed_sequence_value(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    element: &'static str,
    fallback: u32,
) -> Result<u32, MaterialEffectsParseError> {
    attribute_value(reader, event, "Index")?
        .map_or(Ok(fallback), |value| parse_u32(element, "Index", &value))
}

fn attribute_value(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    name: &str,
) -> Result<Option<String>, MaterialEffectsParseError> {
    for attribute in attributes(reader, event)? {
        if attribute.name == name {
            return Ok(Some(attribute.value));
        }
    }
    Ok(None)
}

fn attributes(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<Vec<XmlAttribute>, MaterialEffectsParseError> {
    let mut attributes = Vec::new();
    for attribute in event.attributes() {
        let attribute = attribute?;
        let name = local_name(attribute.key.as_ref());
        let value = attribute
            .decoded_and_normalized_value(quick_xml::XmlVersion::default(), reader.decoder())?
            .into_owned();
        attributes.push(XmlAttribute { name, value });
    }
    Ok(attributes)
}

fn element_name(event: &BytesStart<'_>) -> String {
    local_name(event.name().as_ref())
}

fn local_name(raw: &[u8]) -> String {
    let name = String::from_utf8_lossy(raw);
    name.rsplit(':').next().unwrap_or(&name).to_string()
}

fn parse_u32(
    element: &'static str,
    attribute: &'static str,
    value: &str,
) -> Result<u32, MaterialEffectsParseError> {
    value
        .trim()
        .parse()
        .map_err(|source| MaterialEffectsParseError::InvalidInteger {
            element,
            attribute,
            value: value.to_string(),
            source,
        })
}
