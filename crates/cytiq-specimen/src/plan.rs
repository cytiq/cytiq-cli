//! An immutable selection of acquisition IDs and effective protocol definitions.

use crate::{Result, SpecimenLayout, require};
use cytiq_manifest::{Cytometry, RunManifest};
use cytiq_population::{
    ArtifactRef,
    annotations::{AnnotationFormat, ObservationContext},
    artifact::verify_package,
    serialization,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Acquisition {
    pub id: String,
    pub file: ArtifactRef,
    pub raw_events: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelSelection {
    pub panel_id: String,
    pub tree_sha256: String,
    pub panel_profile_sha256: String,
    pub projection_sha256: String,
    pub acquisitions: Vec<Acquisition>,
    pub selected: Option<String>,
    /// Excluded acquisition IDs are retained, never silently dropped.
    pub excluded: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecimenPlan {
    pub schema_version: u32,
    pub specimen_id: String,
    pub manifest: ArtifactRef,
    pub layout_file: ArtifactRef,
    pub cytometry: Cytometry,
    pub layout: SpecimenLayout,
    pub selections: Vec<PanelSelection>,
}

impl SpecimenPlan {
    /// Plan one specimen without evaluating events or choosing a QC winner.
    /// # Errors
    /// Rejects ambiguous selections, missing files and incompatible modality/source combinations.
    /// # Panics
    /// Never: projection fields are read only after upstream-schema validation.
    pub fn build(
        manifest_path: &Path,
        layout_path: &Path,
        specimen_id: &str,
        selected: &BTreeMap<String, String>,
    ) -> Result<Self> {
        let manifest = RunManifest::load(manifest_path)?;
        let layout = SpecimenLayout::load(layout_path)?;
        require(
            !specimen_id.trim().is_empty()
                && manifest
                    .samples
                    .iter()
                    .any(|s| s.specimen.as_deref() == Some(specimen_id)),
            "unknown specimen id",
        )?;
        require(
            selected.keys().all(|name| layout.panels.contains_key(name)),
            "selection names an unknown panel",
        )?;
        let mut selections = Vec::new();
        for (name, panel) in &layout.panels {
            require(
                manifest.panels.iter().any(|p| p.label() == name),
                "layout panel is absent from run definitions",
            )?;
            require(
                manifest.cytometry != Cytometry::Spectral
                    || panel
                        .sources
                        .values()
                        .all(|s| s.format == AnnotationFormat::GateMembership),
                "spectral classify requires a future validated recipe",
            )?;
            require(
                manifest.cytometry != Cytometry::Cytof
                    || panel
                        .sources
                        .values()
                        .all(|s| s.format == AnnotationFormat::ClassifyPredictions),
                "P1 CyTOF path uses existing classify",
            )?;
            let mut acquisitions = Vec::new();
            for sample in manifest.samples.iter().filter(|s| {
                s.specimen.as_deref() == Some(specimen_id)
                    && manifest.panel_of(s).is_some_and(|p| p.label() == name)
            }) {
                let file = ArtifactRef::of(&sample.file)?;
                let metadata = cytiq_fcs::FcsReader::open(&sample.file)?
                    .stream(
                        std::num::NonZeroUsize::MIN,
                        cytiq_fcs::FcsReadLimits::default(),
                    )?
                    .metadata;
                require(
                    cytiq_fcs::acquisition_id(&sample.file, &metadata.text)? == file.sha256,
                    "manifest samples must name raw acquisition files",
                )?;
                acquisitions.push(Acquisition {
                    id: sample.id.clone(),
                    file,
                    raw_events: metadata.event_count as u64,
                });
            }
            let selection = select_acquisition(name, selected.get(name), &acquisitions)?;
            let excluded = acquisitions
                .iter()
                .filter(|a| Some(&a.id) != selection.as_ref())
                .map(|a| (a.id.clone(), "not_selected".into()))
                .collect();
            let projection = panel.projection.load()?;
            selections.push(PanelSelection {
                panel_id: name.clone(),
                acquisitions,
                selected: selection,
                excluded,
                tree_sha256: projection.value()["tree_sha256"]
                    .as_str()
                    .expect("validated hash")
                    .into(),
                panel_profile_sha256: projection.value()["panel_sha256"]
                    .as_str()
                    .expect("validated hash")
                    .into(),
                projection_sha256: projection.sha256().to_owned(),
            });
        }
        Ok(Self {
            schema_version: 1,
            specimen_id: specimen_id.to_owned(),
            manifest: ArtifactRef::of(manifest_path)?,
            layout_file: ArtifactRef::of(layout_path)?,
            cytometry: manifest.cytometry,
            layout,
            selections,
        })
    }

    /// Read only a package whose completion marker covers this exact plan.
    /// # Errors
    /// Rejects corrupt plans and invalid selection references.
    pub fn load(path: &Path) -> Result<Self> {
        let parent = path.parent().unwrap_or(Path::new("."));
        let outcome = verify_package(parent)?;
        require(
            outcome.command == "cytiq specimen plan"
                && path.file_name().is_some_and(|n| n == "plan.json")
                && outcome.artifacts.iter().any(|a| a.path == "plan.json"),
            "not a specimen plan package",
        )?;
        let plan: Self = serde_json::from_value(serialization::read(path)?)?;
        require(
            plan.schema_version == 1
                && !plan.specimen_id.is_empty()
                && plan.selections.len() == plan.layout.panels.len(),
            "invalid plan schema/selection coverage",
        )?;
        let mut seen = std::collections::BTreeSet::new();
        for selection in &plan.selections {
            require(
                seen.insert(&selection.panel_id)
                    && plan.layout.panels.contains_key(&selection.panel_id),
                "invalid plan panel selection",
            )?;
            require(
                selection
                    .selected
                    .as_ref()
                    .is_none_or(|id| selection.acquisitions.iter().any(|a| &a.id == id)),
                "dangling selected acquisition",
            )?;
        }
        Ok(plan)
    }

    /// Build the population adapter's context without a reverse crate dependency.
    /// # Errors
    /// Rejects unselected acquisition IDs and unknown sources.
    pub fn context(&self, acquisition: &str, source: &str) -> Result<ObservationContext> {
        for selection in &self.selections {
            if selection.selected.as_deref() == Some(acquisition) {
                let sample = selection
                    .acquisitions
                    .iter()
                    .find(|a| a.id == acquisition)
                    .ok_or_else(|| {
                        crate::SpecimenError::Invalid("dangling selected acquisition".into())
                    })?;
                let panel = self.layout.panels.get(&selection.panel_id).ok_or_else(|| {
                    crate::SpecimenError::Invalid("dangling selected panel".into())
                })?;
                let definition = panel
                    .sources
                    .get(source)
                    .ok_or_else(|| crate::SpecimenError::Invalid("unknown source id".into()))?;
                return Ok(ObservationContext {
                    specimen_id: self.specimen_id.clone(),
                    manifest_sha256: self.manifest.sha256.clone(),
                    panel_id: selection.panel_id.clone(),
                    acquisition_id: acquisition.into(),
                    acquisition_sha256: sample.file.sha256.clone(),
                    raw_events: sample.raw_events,
                    source_id: source.into(),
                    projection: panel.projection.clone(),
                    source: definition.clone(),
                });
            }
        }
        Err(crate::SpecimenError::Invalid(
            "acquisition is not selected in this plan".into(),
        ))
    }

    #[must_use]
    pub fn missing_required(&self) -> bool {
        self.selections
            .iter()
            .any(|s| s.selected.is_none() && self.layout.panels[&s.panel_id].required)
    }

    #[must_use]
    pub fn input_files(&self) -> Vec<PathBuf> {
        let mut files = vec![self.manifest.path.clone(), self.layout_file.path.clone()];
        files.extend(self.layout.input_files());
        files.extend(
            self.selections
                .iter()
                .flat_map(|s| s.acquisitions.iter().map(|a| a.file.path.clone())),
        );
        files
    }
}

fn select_acquisition(
    name: &str,
    selected: Option<&String>,
    acquisitions: &[Acquisition],
) -> Result<Option<String>> {
    let selection = match (selected, acquisitions) {
        (Some(id), _) => {
            require(
                acquisitions.iter().any(|a| &a.id == id),
                "selected acquisition is not in the specimen panel",
            )?;
            Some(id.clone())
        }
        (None, []) => None,
        (None, [acquisition]) => Some(acquisition.id.clone()),
        (None, _) => {
            return Err(crate::SpecimenError::Invalid(format!(
                "panel `{name}` requires --select PANEL=ACQUISITION"
            )));
        }
    };
    Ok(selection)
}
