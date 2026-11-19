//! Turning `ReportData` (read responses, subscription priming and
//! subscription reports) into [`AttributeValue`]s and [`EventValue`]s.

use matter_model::Shape;
use rs_matter::im::{AttrResp, EventResp, ReportDataResp};
use serde_json::Value;

use crate::{AttributeValue, EventValue};

/// Iterate a TLV container a peer sent, after checking that it is one: a
/// malformed message is refused rather than trusted.
pub(crate) fn items<'a, T, C>(
    c: &rs_matter::tlv::TLVContainer<'a, T, C>,
) -> Result<rs_matter::tlv::TLVContainerIter<'a, T>, rs_matter::error::Error>
where
    T: rs_matter::tlv::FromTLV<'a>,
{
    c.element().container()?;
    Ok(c.iter())
}

/// What one report (all of its chunks) carried.
#[derive(Default)]
pub(crate) struct Collected {
    pub attributes: Vec<AttributeValue>,
    pub events: Vec<EventValue>,
}

/// Decode one `ReportData` message payload from `node` the way the
/// controller does: for tests, recorded sessions and fuzzing.
pub fn decode_report_data(
    node: u64,
    payload: &[u8],
) -> Result<(Vec<AttributeValue>, Vec<EventValue>), crate::Error> {
    use rs_matter::tlv::{FromTLV, TLVElement};
    let resp = ReportDataResp::from_tlv(&TLVElement::new(payload))?;
    let mut collected = Collected::default();
    collected.add_chunk(node, &resp);
    Ok((collected.attributes, collected.events))
}

impl Collected {
    /// Add one chunk of a report from `node`.
    ///
    /// A list too long for one message arrives as an empty list followed by
    /// "append" items (a path with a null list index), possibly over several
    /// chunks; they are put back together here, so every value delivered is a
    /// whole list.
    pub fn add_chunk(&mut self, node: u64, resp: &ReportDataResp<'_>) {
        let attr_reports = resp.attr_reports.as_ref().map(items).transpose();
        let event_reports = resp.event_reports.as_ref().map(items).transpose();
        let (attr_reports, event_reports) = match (attr_reports, event_reports) {
            (Ok(a), Ok(e)) => (a, e),
            _ => {
                log::warn!("node {node:016X}: malformed report, not a list of reports");
                return;
            }
        };

        if let Some(reports) = attr_reports {
            for report in reports {
                match report {
                    Ok(AttrResp::Data(data)) => {
                        let (Some(endpoint), Some(cluster), Some(attribute)) =
                            (data.path.endpoint, data.path.cluster, data.path.attr)
                        else {
                            continue;
                        };
                        let attr =
                            matter_model::cluster(cluster).and_then(|c| c.attribute(attribute));
                        let append = data.path.list_index.as_ref().is_some_and(|i| i.is_none());
                        let shape = match (attr, append) {
                            (Some(a), true) => Shape::of(a.ty),
                            (Some(a), false) => a.shape(),
                            (None, _) => Shape::ANY,
                        };
                        let value = match matter_model::to_json(&data.data, shape) {
                            Ok(v) => v,
                            Err(e) => {
                                log::warn!(
                                    "node {node:016X}: undecodable {endpoint}/{cluster:#06x}/{attribute:#06x}: {e}"
                                );
                                continue;
                            }
                        };
                        if append {
                            match self.attributes.iter_mut().rev().find(|a| {
                                a.endpoint == endpoint
                                    && a.cluster == cluster
                                    && a.attribute == attribute
                            }) {
                                Some(AttributeValue {
                                    value: Value::Array(items),
                                    ..
                                }) => items.push(value),
                                _ => self.attributes.push(AttributeValue {
                                    node,
                                    endpoint,
                                    cluster,
                                    attribute,
                                    value: Value::Array(vec![value]),
                                    status: None,
                                }),
                            }
                        } else {
                            self.attributes.push(AttributeValue {
                                node,
                                endpoint,
                                cluster,
                                attribute,
                                value,
                                status: None,
                            });
                        }
                    }
                    Ok(AttrResp::Status(status)) => {
                        let path = status.path;
                        self.attributes.push(AttributeValue {
                            node,
                            endpoint: path.endpoint.unwrap_or_default(),
                            cluster: path.cluster.unwrap_or_default(),
                            attribute: path.attr.unwrap_or_default(),
                            value: Value::Null,
                            status: Some(format!("{:?}", status.status.status)),
                        });
                    }
                    Err(e) => log::warn!("node {node:016X}: malformed attribute report: {e:?}"),
                }
            }
        }

        if let Some(reports) = event_reports {
            for report in reports {
                match report {
                    Ok(EventResp::Data(data)) => {
                        let (Some(endpoint), Some(cluster), Some(event)) =
                            (data.path.endpoint, data.path.cluster, data.path.event)
                        else {
                            continue;
                        };
                        let shape = matter_model::cluster(cluster)
                            .and_then(|c| c.event(event))
                            .map_or(Shape::ANY, |e| e.shape());
                        match matter_model::to_json(&data.data, shape) {
                            Ok(value) => self.events.push(EventValue {
                                node,
                                endpoint,
                                cluster,
                                event,
                                number: data.event_number,
                                data: value,
                            }),
                            Err(e) => log::warn!(
                                "node {node:016X}: undecodable event {endpoint}/{cluster:#06x}/{event:#06x}: {e}"
                            ),
                        }
                    }
                    Ok(EventResp::Status(status)) => log::debug!(
                        "node {node:016X}: event status {:?} for {:?}",
                        status.status.status,
                        status.path
                    ),
                    Err(e) => log::warn!("node {node:016X}: malformed event report: {e:?}"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_whose_reports_are_not_a_list_is_refused() {
        // AttributeReportIBs (tag 1) as an unsigned integer.
        let (attributes, events) = decode_report_data(1, &[0x15, 0x24, 0x01, 0x05, 0x18]).unwrap();
        assert!(attributes.is_empty() && events.is_empty());
    }
}
