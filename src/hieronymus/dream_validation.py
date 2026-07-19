from __future__ import annotations

import re
from dataclasses import dataclass, field

from hieronymus.concepts import VALID_FACET_KINDS
from hieronymus.memory_models import ShortTermMemoryRecord, TranslationContext
from hieronymus.values import clamp_score as _clamp_score
from hieronymus.values import normalize_string_tuple

ALLOWED_CRYSTAL_TYPES = frozenset(
    {"lesson", "rule", "thought", "observation", "concept_note", "concept", "erudition"}
)
MALFORMED_CONFIDENCE_PENALTY = 0.2
SOURCE_CREDIBILITY_CONFIDENCE = {
    "rumor": 0.15,
    "observation": 0.35,
    "source_text": 0.7,
    "expert": 0.85,
    "user_suggestion": 0.8,
    "user_rule": 0.95,
    "thought": 0.2,
}
_MIN_NORMALIZED_CONFIDENCE = 0.05
_SENTENCE_RE = re.compile(r"[^.!?。！？]+[.!?。！？]?")


@dataclass(frozen=True)
class DreamCrystalCandidate:
    crystal_type: str
    title: str
    text: str
    strength: float
    confidence: float
    source_memory_ids: list[int] = field(default_factory=list)
    source_credibility: str = "observation"
    rule_intent: str = ""
    is_inferred: bool = False


@dataclass(frozen=True)
class DreamConceptProposal:
    series_slug: str
    source_language: str
    target_language: str
    concept_text: str
    source_form: str
    canonical_rendering: str
    approved_variants: list[str] = field(default_factory=list)
    forbidden_variants: list[str] = field(default_factory=list)
    rationale: str = ""


@dataclass(frozen=True)
class DreamOutput:
    crystals: list[DreamCrystalCandidate] = field(default_factory=list)
    concept_proposals: list[DreamConceptProposal] = field(default_factory=list)


@dataclass(frozen=True)
class DreamParseWarning:
    entry_path: str
    code: str
    message: str
    confidence_penalty: float


@dataclass(frozen=True)
class NormalizedDreamCrystal:
    crystal_type: str
    title: str
    text: str
    strength: float
    confidence: float
    source_memory_ids: list[int]
    source_credibility: str = "observation"
    rule_intent: str = ""
    malformed_penalty: float = 0.0
    is_inferred: bool = False
    supersedes_crystal_id: int | None = None
    story_scopes: tuple[str, ...] = ()
    semantic_tags: tuple[str, ...] = ()
    concept_ids: tuple[int, ...] = ()
    concept_names: tuple[str, ...] = ()


@dataclass(frozen=True)
class NormalizedDreamConcept:
    canonical_name: str
    description: str = ""
    tags: tuple[str, ...] = ()
    confidence_delta: float = 0.2


@dataclass(frozen=True)
class NormalizedDreamFacet:
    concept_name: str
    value: str
    kind: str = "note"
    language_tags: tuple[str, ...] = ()
    story_scopes: tuple[str, ...] = ()
    semantic_tags: tuple[str, ...] = ()
    confidence: float = 0.2
    is_canonical: bool = False


@dataclass(frozen=True)
class DreamSupersedeAction:
    old_crystal_id: int
    new_crystal_id: int
    reason: str = ""


@dataclass(frozen=True)
class NormalizedDreamOutput:
    crystals: list[NormalizedDreamCrystal] = field(default_factory=list)
    concept_proposals: list[DreamConceptProposal] = field(default_factory=list)
    concepts: list[NormalizedDreamConcept] = field(default_factory=list)
    facets: list[NormalizedDreamFacet] = field(default_factory=list)
    supersede_actions: list[DreamSupersedeAction] = field(default_factory=list)
    reinforce_actions: list[tuple[int, float, float]] = field(default_factory=list)
    warnings: list[DreamParseWarning] = field(default_factory=list)
    rejected_entries: list[dict[str, object]] = field(default_factory=list)
    skipped_candidates: list[dict[str, object]] = field(default_factory=list)


_NormalizedDreamCrystal = NormalizedDreamCrystal
_NormalizedDreamConcept = NormalizedDreamConcept
_NormalizedDreamFacet = NormalizedDreamFacet
_DreamSupersedeAction = DreamSupersedeAction
_NormalizedDreamOutput = NormalizedDreamOutput
_ALLOWED_CRYSTAL_TYPES = ALLOWED_CRYSTAL_TYPES


def normalize_output(
    raw_output: object,
    context: TranslationContext,
    allowed_memory_ids: set[int],
    valid_concept_ids: set[int],
) -> NormalizedDreamOutput:
    if isinstance(raw_output, DreamOutput):
        validate_output(raw_output, context, allowed_memory_ids)
        return NormalizedDreamOutput(
            crystals=[
                NormalizedDreamCrystal(
                    crystal_type=candidate.crystal_type,
                    title=candidate.title,
                    text=candidate.text,
                    strength=candidate.strength,
                    confidence=candidate.confidence,
                    source_memory_ids=candidate.source_memory_ids,
                    source_credibility=candidate.source_credibility,
                    rule_intent=candidate.rule_intent,
                    is_inferred=candidate.is_inferred,
                )
                for candidate in raw_output.crystals
            ],
            concept_proposals=raw_output.concept_proposals,
        )
    if type(raw_output) is not dict:
        raise ValueError("provider output must be a DreamOutput or dict")
    return normalize_dict_output(raw_output, allowed_memory_ids, valid_concept_ids)


def normalize_dict_output(
    payload: dict[object, object],
    allowed_memory_ids: set[int],
    valid_concept_ids: set[int],
) -> NormalizedDreamOutput:
    crystals: list[NormalizedDreamCrystal] = []
    concepts: list[NormalizedDreamConcept] = []
    facets: list[NormalizedDreamFacet] = []
    warnings: list[DreamParseWarning] = []
    skipped_candidates: list[dict[str, object]] = []

    for index, item in enumerate(_list_from_payload(payload.get("concepts"))):
        concept = _normalize_dict_concept(item, f"concepts[{index}]", warnings)
        if concept is not None:
            concepts.append(concept)
    for index, item in enumerate(_list_from_payload(payload.get("facets"))):
        facet = _normalize_dict_facet(item, f"facets[{index}]", warnings)
        if facet is not None:
            facets.append(facet)

    collections = (
        ("crystals", "observation", "observation", False, False),
        ("rule_crystals", "rule", "user_rule", False, False),
        ("thoughts", "thought", "thought", True, True),
        ("inferred_additions", "thought", "thought", True, True),
    )
    for collection, crystal_type, credibility, force_thought, force_inferred in collections:
        for index, item in enumerate(_list_from_payload(payload.get(collection))):
            crystal = _normalize_dict_crystal(
                item,
                entry_path=f"{collection}[{index}]",
                warnings=warnings,
                skipped_candidates=skipped_candidates,
                default_crystal_type=crystal_type,
                default_source_credibility=credibility,
                allowed_memory_ids=allowed_memory_ids,
                valid_concept_ids=valid_concept_ids,
                force_thought=force_thought,
                force_inferred=force_inferred,
            )
            if crystal is not None:
                crystals.append(crystal)

    supersede_actions = []
    for index, item in enumerate(_list_from_payload(payload.get("supersede"))):
        action = _normalize_supersede_action(item)
        if action is None:
            skipped_candidates.append(
                {
                    "entry_path": f"supersede[{index}]",
                    "reason": "malformed_supersede_action",
                    "candidate_type": type(item).__name__,
                }
            )
            continue
        supersede_actions.append(action)
    reinforce_actions = []
    for item in _list_from_payload(payload.get("reinforce")):
        if type(item) is not dict:
            raise ValueError("reinforce actions must be objects")
        source_memory_ids = item.get("source_memory_ids")
        if (
            type(source_memory_ids) is not list
            or not source_memory_ids
            or not all(type(memory_id) is int for memory_id in source_memory_ids)
            or not set(source_memory_ids).issubset(allowed_memory_ids)
        ):
            raise ValueError("reinforce actions require selected source_memory_ids")
        action = _normalize_score_action(item)
        if action is None:
            raise ValueError("reinforce actions must contain crystal_id and numeric deltas")
        reinforce_actions.append(action)
    return NormalizedDreamOutput(
        crystals=crystals,
        concepts=concepts,
        facets=facets,
        supersede_actions=supersede_actions,
        reinforce_actions=reinforce_actions,
        warnings=warnings,
        skipped_candidates=skipped_candidates,
    )


def validate_normalized_output(
    output: NormalizedDreamOutput,
    context: TranslationContext,
    allowed_memory_ids: set[int],
) -> None:
    for candidate in output.crystals:
        if candidate.crystal_type not in ALLOWED_CRYSTAL_TYPES:
            raise ValueError(f"unknown crystal_type: {candidate.crystal_type}")
        if not candidate.text.strip():
            raise ValueError("candidate text must not be empty")
        if not 0.0 <= candidate.strength <= 1.0:
            raise ValueError("candidate strength must be between 0 and 1")
        if not 0.0 <= candidate.confidence <= 1.0:
            raise ValueError("candidate confidence must be between 0 and 1")
        if candidate.source_memory_ids:
            unknown_ids = set(candidate.source_memory_ids) - allowed_memory_ids
            if unknown_ids:
                raise ValueError(f"unknown source_memory_ids: {sorted(unknown_ids)}")

    for proposal in output.concept_proposals:
        if proposal.series_slug != context.series_slug:
            raise ValueError("proposal series_slug must match context")
        if proposal.source_language != context.source_language:
            raise ValueError("proposal source_language must match context")
        if proposal.target_language != context.target_language:
            raise ValueError("proposal target_language must match context")

    for facet in output.facets:
        if not facet.concept_name.strip():
            raise ValueError("facet concept_name must not be empty")
        if not facet.value.strip():
            raise ValueError("facet value must not be empty")
        if facet.kind not in VALID_FACET_KINDS:
            raise ValueError(f"unknown facet kind: {facet.kind}")
        if not 0.0 <= facet.confidence <= 1.0:
            raise ValueError("facet confidence must be between 0 and 1")


def validate_output(
    output: DreamOutput,
    context: TranslationContext,
    allowed_memory_ids: set[int],
) -> None:
    if not isinstance(output, DreamOutput):
        raise ValueError("provider output must be a DreamOutput")
    if not isinstance(output.crystals, list):
        raise ValueError("provider output crystals must be a list")
    if not isinstance(output.concept_proposals, list):
        raise ValueError("provider output concept_proposals must be a list")

    for candidate in output.crystals:
        if not isinstance(candidate, DreamCrystalCandidate):
            raise ValueError("provider crystals must contain only DreamCrystalCandidate items")
        if candidate.crystal_type not in ALLOWED_CRYSTAL_TYPES:
            raise ValueError(f"unknown crystal_type: {candidate.crystal_type}")
        if not candidate.text.strip():
            raise ValueError("candidate text must not be empty")
        if not 0.0 <= candidate.strength <= 1.0:
            raise ValueError("candidate strength must be between 0 and 1")
        if not 0.0 <= candidate.confidence <= 1.0:
            raise ValueError("candidate confidence must be between 0 and 1")
        if not candidate.source_memory_ids:
            raise ValueError("candidate source_memory_ids must not be empty")
        unknown_ids = set(candidate.source_memory_ids) - allowed_memory_ids
        if unknown_ids:
            raise ValueError(f"unknown source_memory_ids: {sorted(unknown_ids)}")

    for proposal in output.concept_proposals:
        if not isinstance(proposal, DreamConceptProposal):
            raise ValueError(
                "provider concept_proposals must contain only DreamConceptProposal items"
            )
        if proposal.series_slug != context.series_slug:
            raise ValueError("proposal series_slug must match context")
        if proposal.source_language != context.source_language:
            raise ValueError("proposal source_language must match context")
        if proposal.target_language != context.target_language:
            raise ValueError("proposal target_language must match context")
        if not isinstance(proposal.concept_text, str):
            raise ValueError("proposal concept_text must be a string")
        if not isinstance(proposal.source_form, str):
            raise ValueError("proposal source_form must be a string")
        if not isinstance(proposal.canonical_rendering, str):
            raise ValueError("proposal canonical_rendering must be a string")
        if not proposal.concept_text.strip():
            raise ValueError("proposal concept_text must not be empty")
        if not proposal.source_form.strip():
            raise ValueError("proposal source_form must not be empty")
        if not proposal.canonical_rendering.strip():
            raise ValueError("proposal canonical_rendering must not be empty")
        if not isinstance(proposal.approved_variants, list):
            raise ValueError("proposal approved_variants must be a list")
        if not all(isinstance(variant, str) for variant in proposal.approved_variants):
            raise ValueError("proposal approved_variants must contain only strings")
        if not isinstance(proposal.forbidden_variants, list):
            raise ValueError("proposal forbidden_variants must be a list")
        if not all(isinstance(variant, str) for variant in proposal.forbidden_variants):
            raise ValueError("proposal forbidden_variants must contain only strings")


def _normalize_candidate_text(text: str) -> str:
    chunks = [match.group(0).strip() for match in _SENTENCE_RE.finditer(text)]
    chunks = [chunk for chunk in chunks if chunk]
    if not chunks:
        return " ".join(text.split())
    return " ".join(chunks[:3])


def _normalize_dict_crystal(
    item: object,
    *,
    entry_path: str,
    warnings: list[DreamParseWarning],
    skipped_candidates: list[dict[str, object]],
    default_crystal_type: str,
    default_source_credibility: str,
    allowed_memory_ids: set[int],
    valid_concept_ids: set[int],
    force_thought: bool = False,
    force_inferred: bool = False,
) -> _NormalizedDreamCrystal | None:
    payload: dict[object, object]
    if isinstance(item, str):
        payload = {"text": item}
    elif type(item) is dict:
        payload = item
    else:
        skipped_candidates.append(
            {
                "entry_path": entry_path,
                "reason": "malformed_candidate",
                "candidate_type": type(item).__name__,
            }
        )
        return None

    text, penalty = _recover_crystal_text(payload)
    if penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_content_field",
            "used fallback content field",
            penalty,
        )
    crystal_type, kind_penalty = _recover_crystal_type(payload, default_crystal_type)
    if force_thought:
        crystal_type = "thought"
    penalty += kind_penalty
    if kind_penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_crystal_type",
            "used fallback crystal type",
            kind_penalty,
        )
    source_credibility, credibility_penalty = _recover_source_credibility(
        payload,
        default_source_credibility,
    )
    if force_thought:
        source_credibility = "thought"
    penalty += credibility_penalty
    if credibility_penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_source_credibility",
            "used fallback source credibility",
            credibility_penalty,
        )
    penalty += max(_numeric_field(payload.get("malformed_penalty"), default=0.0), 0.0)

    is_inferred = force_inferred or _bool_field(payload.get("is_inferred"), default=False)
    if is_inferred and not force_thought:
        crystal_type = "thought"
        source_credibility = "thought"
    story_scopes, story_scope_penalty = _recover_crystal_string_tuple(
        payload,
        entry_path,
        warnings,
        "malformed_crystal_story_scopes",
        "ignored malformed crystal story scope metadata",
        "story_scopes",
    )
    semantic_tags, semantic_tag_penalty = _recover_crystal_string_tuple(
        payload,
        entry_path,
        warnings,
        "malformed_crystal_semantic_tags",
        "ignored malformed crystal semantic tag metadata",
        "semantic_tags",
        "tags",
    )
    concept_ids, concept_id_penalty = _recover_crystal_int_tuple(
        payload,
        entry_path,
        warnings,
        "malformed_crystal_concept_ids",
        "ignored malformed crystal concept id metadata",
        "concept_ids",
        "concept_id",
        valid_ids=valid_concept_ids,
    )
    concept_names, concept_name_penalty = _concept_names_from_payload(payload, entry_path, warnings)
    penalty += (
        story_scope_penalty + semantic_tag_penalty + concept_id_penalty + concept_name_penalty
    )
    confidence = _normalized_confidence(payload, source_credibility, penalty)
    title = _string_field(payload.get("title")).strip() or _title_from_kind(crystal_type)
    source_memory_ids = _source_memory_ids(payload, allowed_memory_ids)
    if source_memory_ids is None:
        skipped_candidates.append(
            {
                "entry_path": entry_path,
                "reason": "invalid_source_memory_ids",
                "source_memory_ids": list(_raw_source_memory_ids(payload)),
            }
        )
        return None
    return _NormalizedDreamCrystal(
        crystal_type=crystal_type,
        title=title,
        text=text,
        strength=_clamp_score(_numeric_field(payload.get("strength"), default=0.5)),
        confidence=confidence,
        source_memory_ids=source_memory_ids,
        source_credibility=source_credibility,
        rule_intent=_string_field(payload.get("rule_intent")),
        malformed_penalty=penalty,
        is_inferred=is_inferred,
        supersedes_crystal_id=_optional_int(payload.get("supersedes_crystal_id")),
        story_scopes=story_scopes,
        semantic_tags=semantic_tags,
        concept_ids=concept_ids,
        concept_names=concept_names,
    )


def _recover_crystal_text(payload: dict[object, object]) -> tuple[str, float]:
    for key in ("content", "text"):
        value = payload.get(key)
        if isinstance(value, str) and value.strip():
            return (" ".join(value.split()), 0.0)
    value = payload.get("body")
    if isinstance(value, str) and value.strip():
        return (" ".join(value.split()), MALFORMED_CONFIDENCE_PENALTY)
    raise ValueError("dream candidate content is required")


def _recover_crystal_type(
    payload: dict[object, object],
    default_crystal_type: str,
) -> tuple[str, float]:
    penalty = 0.0
    if "crystal_type" in payload:
        value = payload["crystal_type"]
    elif "type" in payload:
        value = payload["type"]
    elif "kind" in payload:
        value = payload["kind"]
    else:
        return (default_crystal_type, 0.0)

    if not isinstance(value, str):
        return ("observation", penalty + MALFORMED_CONFIDENCE_PENALTY)

    crystal_type = value.strip().casefold().replace("-", "_")
    if crystal_type == "rule_crystal":
        return ("rule", penalty + MALFORMED_CONFIDENCE_PENALTY)
    if crystal_type in _ALLOWED_CRYSTAL_TYPES:
        return (crystal_type, penalty)
    return ("observation", penalty + MALFORMED_CONFIDENCE_PENALTY)


def _recover_source_credibility(
    payload: dict[object, object],
    default_source_credibility: str,
) -> tuple[str, float]:
    value = payload.get("source_credibility", default_source_credibility)
    if isinstance(value, str) and value.strip():
        return (value.strip(), 0.0)
    return (default_source_credibility, MALFORMED_CONFIDENCE_PENALTY)


def _normalized_confidence(
    payload: dict[object, object],
    source_credibility: str,
    penalty: float,
) -> float:
    if source_credibility in SOURCE_CREDIBILITY_CONFIDENCE:
        base = SOURCE_CREDIBILITY_CONFIDENCE[source_credibility]
    else:
        base = _numeric_field(
            payload.get("confidence"), default=SOURCE_CREDIBILITY_CONFIDENCE["observation"]
        )
    return min(max(base - penalty, _MIN_NORMALIZED_CONFIDENCE), 1.0)


def _normalize_dict_concept(
    item: object,
    entry_path: str,
    warnings: list[DreamParseWarning],
) -> _NormalizedDreamConcept | None:
    if type(item) is not dict:
        return None
    payload: dict[object, object] = item
    penalty = 0.0
    name = _string_field(payload.get("canonical_name")).strip()
    if name == "":
        name = _string_field(payload.get("name")).strip()
    if name == "":
        name = _string_field(payload.get("label")).strip()
        if name:
            penalty += MALFORMED_CONFIDENCE_PENALTY
            _append_parse_warning(
                warnings,
                entry_path,
                "malformed_concept_name",
                "used fallback concept label",
                MALFORMED_CONFIDENCE_PENALTY,
            )
    if name == "":
        raise ValueError(f"{entry_path}.canonical_name is required")
    confidence = _numeric_field(payload.get("confidence"), default=0.2)
    confidence_delta = min(max(confidence - penalty, _MIN_NORMALIZED_CONFIDENCE), 1.0)
    return _NormalizedDreamConcept(
        canonical_name=name,
        description=_string_field(payload.get("description")),
        tags=_clean_string_tuple(payload.get("tags"), payload.get("semantic_tags")),
        confidence_delta=confidence_delta,
    )


def _normalize_dict_facet(
    item: object,
    entry_path: str,
    warnings: list[DreamParseWarning],
) -> _NormalizedDreamFacet | None:
    if type(item) is not dict:
        return None
    payload: dict[object, object] = item
    value, content_penalty = _recover_facet_value(payload)
    if value is None:
        raise ValueError(f"{entry_path}.value is required")
    if content_penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_facet_value",
            "used fallback facet value field",
            content_penalty,
        )

    concept_name = _facet_concept_name_from_payload(payload, entry_path, warnings)
    if concept_name == "":
        raise ValueError(f"{entry_path}.concept_name is required")

    kind, kind_penalty = _recover_facet_kind(payload)
    language_tags, language_penalty = _recover_facet_string_tuple(
        payload,
        "language_tags",
        "language",
    )
    story_scopes, story_scope_penalty = _recover_facet_string_tuple(
        payload,
        "story_scopes",
        "story_scope",
    )
    semantic_tags, semantic_tag_penalty = _recover_facet_string_tuple(
        payload,
        "semantic_tags",
        "tags",
    )
    is_canonical, canonical_penalty = _recover_facet_canonical(payload)
    metadata_penalty = (
        kind_penalty
        + content_penalty
        + language_penalty
        + story_scope_penalty
        + semantic_tag_penalty
        + canonical_penalty
    )
    if kind_penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_facet_kind",
            "used fallback facet kind",
            kind_penalty,
        )
    if language_penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_facet_language_tags",
            "ignored malformed facet language metadata",
            language_penalty,
        )
    if story_scope_penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_facet_story_scopes",
            "ignored malformed facet story scope metadata",
            story_scope_penalty,
        )
    if semantic_tag_penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_facet_semantic_tags",
            "ignored malformed facet semantic tag metadata",
            semantic_tag_penalty,
        )
    if canonical_penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_facet_canonical",
            "parsed non-boolean canonical metadata",
            canonical_penalty,
        )
    confidence = _numeric_field(payload.get("confidence"), default=0.2)
    return _NormalizedDreamFacet(
        concept_name=concept_name,
        value=value,
        kind=kind,
        language_tags=language_tags,
        story_scopes=story_scopes,
        semantic_tags=semantic_tags,
        confidence=min(max(confidence - metadata_penalty, _MIN_NORMALIZED_CONFIDENCE), 1.0),
        is_canonical=is_canonical,
    )


def _recover_facet_value(payload: dict[object, object]) -> tuple[str | None, float]:
    for key in ("content", "value", "text"):
        value = payload.get(key)
        if isinstance(value, str) and value.strip():
            return (" ".join(value.split()), 0.0)
    value = payload.get("body")
    if isinstance(value, str) and value.strip():
        return (" ".join(value.split()), MALFORMED_CONFIDENCE_PENALTY)
    return (None, 0.0)


def _recover_facet_kind(payload: dict[object, object]) -> tuple[str, float]:
    if "kind" not in payload and "facet_type" not in payload:
        return ("note", 0.0)
    value = payload.get("kind", payload.get("facet_type"))
    if not isinstance(value, str) or not value.strip():
        return ("note", MALFORMED_CONFIDENCE_PENALTY)
    clean_kind = value.strip().casefold().replace("-", "_")
    if clean_kind in VALID_FACET_KINDS:
        return (clean_kind, 0.0)
    if clean_kind in {"alias", "former_label"}:
        return ("name", MALFORMED_CONFIDENCE_PENALTY)
    return ("note", MALFORMED_CONFIDENCE_PENALTY)


def _recover_facet_string_tuple(
    payload: dict[object, object],
    *keys: str,
) -> tuple[tuple[str, ...], float]:
    values: list[str] = []
    penalty = 0.0
    for key in keys:
        if key not in payload:
            continue
        value = payload[key]
        if isinstance(value, str):
            if value.strip():
                values.append(value)
            else:
                penalty += MALFORMED_CONFIDENCE_PENALTY
            continue
        if type(value) is list:
            for item in value:
                if isinstance(item, str) and item.strip():
                    values.append(item)
                else:
                    penalty += MALFORMED_CONFIDENCE_PENALTY
            continue
        penalty += MALFORMED_CONFIDENCE_PENALTY
    return (_clean_text_tuple(tuple(values)), penalty)


def _recover_facet_canonical(payload: dict[object, object]) -> tuple[bool, float]:
    if "is_canonical" in payload:
        return _parse_facet_canonical(payload["is_canonical"])
    if "canonical" in payload:
        return _parse_facet_canonical(payload["canonical"])
    return (False, 0.0)


def _parse_facet_canonical(value: object) -> tuple[bool, float]:
    if isinstance(value, bool):
        return (value, 0.0)
    if isinstance(value, str):
        clean_value = value.strip().casefold()
        if clean_value in {"true", "yes", "1"}:
            return (True, MALFORMED_CONFIDENCE_PENALTY)
        if clean_value in {"false", "no", "0"}:
            return (False, MALFORMED_CONFIDENCE_PENALTY)
        return (False, MALFORMED_CONFIDENCE_PENALTY)
    if isinstance(value, int) and value in {0, 1}:
        return (bool(value), MALFORMED_CONFIDENCE_PENALTY)
    return (False, MALFORMED_CONFIDENCE_PENALTY)


def _facet_concept_name_from_payload(
    payload: dict[object, object],
    entry_path: str,
    warnings: list[DreamParseWarning],
) -> str:
    for key in ("concept_name", "concept_label", "canonical_name"):
        value = payload.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    concept = payload.get("concept")
    if isinstance(concept, str) and concept.strip():
        return concept.strip()
    if type(concept) is dict:
        normalized = _normalize_dict_concept(concept, f"{entry_path}.concept", warnings)
        if normalized is not None:
            return normalized.canonical_name
    return ""


def _normalize_supersede_action(item: object) -> _DreamSupersedeAction | None:
    if type(item) is not dict:
        return None
    payload: dict[object, object] = item
    old_crystal_id = _optional_int(payload.get("old_crystal_id"))
    new_crystal_id = _optional_int(payload.get("new_crystal_id"))
    if old_crystal_id is None or new_crystal_id is None:
        return None
    return _DreamSupersedeAction(
        old_crystal_id=old_crystal_id,
        new_crystal_id=new_crystal_id,
        reason=_string_field(payload.get("reason")),
    )


def _normalize_score_action(item: object) -> tuple[int, float, float] | None:
    if type(item) is not dict:
        return None
    payload: dict[object, object] = item
    crystal_id = _optional_int(payload.get("crystal_id"))
    if crystal_id is None:
        return None
    return (
        crystal_id,
        _numeric_field(payload.get("strength_delta"), default=0.0),
        _numeric_field(payload.get("confidence_delta"), default=0.0),
    )


def _list_from_payload(value: object) -> list[object]:
    if value is None:
        return []
    if type(value) is list:
        return value
    return [value]


def _string_field(value: object) -> str:
    return value if isinstance(value, str) else ""


def _numeric_field(value: object, *, default: float) -> float:
    if isinstance(value, bool):
        return default
    if isinstance(value, int | float):
        return float(value)
    return default


def _bool_field(value: object, *, default: bool) -> bool:
    if isinstance(value, bool):
        return value
    return default


def _append_parse_warning(
    warnings: list[DreamParseWarning],
    entry_path: str,
    code: str,
    message: str,
    confidence_penalty: float,
) -> None:
    warnings.append(
        DreamParseWarning(
            entry_path=entry_path,
            code=code,
            message=message,
            confidence_penalty=confidence_penalty,
        )
    )


def _optional_int(value: object) -> int | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        try:
            return int(value.strip())
        except ValueError:
            return None
    return None


def _source_memory_ids(
    payload: dict[object, object],
    allowed_memory_ids: set[int],
) -> list[int] | None:
    has_source_field = "source_memory_ids" in payload or "source_memory_id" in payload
    clean_ids = [
        memory_id
        for memory_id in _clean_int_tuple(
            payload.get("source_memory_ids"), payload.get("source_memory_id")
        )
        if memory_id in allowed_memory_ids
    ]
    if clean_ids:
        return clean_ids
    if has_source_field:
        return None
    return sorted(allowed_memory_ids)


def _raw_source_memory_ids(payload: dict[object, object]) -> tuple[int, ...]:
    return _clean_int_tuple(payload.get("source_memory_ids"), payload.get("source_memory_id"))


def _recover_crystal_string_tuple(
    payload: dict[object, object],
    entry_path: str,
    warnings: list[DreamParseWarning],
    code: str,
    message: str,
    *keys: str,
) -> tuple[tuple[str, ...], float]:
    values, penalty = _recover_facet_string_tuple(payload, *keys)
    if penalty:
        _append_parse_warning(warnings, entry_path, code, message, penalty)
    return (values, penalty)


def _recover_crystal_int_tuple(
    payload: dict[object, object],
    entry_path: str,
    warnings: list[DreamParseWarning],
    code: str,
    message: str,
    *keys: str,
    valid_ids: set[int] | None = None,
) -> tuple[tuple[int, ...], float]:
    integers: list[int] = []
    penalty = 0.0
    for key in keys:
        if key not in payload:
            continue
        value = payload[key]
        if isinstance(value, bool):
            penalty += MALFORMED_CONFIDENCE_PENALTY
        elif isinstance(value, int):
            integers.append(value)
        elif type(value) is list:
            for item in value:
                if isinstance(item, bool):
                    penalty += MALFORMED_CONFIDENCE_PENALTY
                elif isinstance(item, int):
                    integers.append(item)
                else:
                    penalty += MALFORMED_CONFIDENCE_PENALTY
        else:
            penalty += MALFORMED_CONFIDENCE_PENALTY
    if penalty:
        _append_parse_warning(warnings, entry_path, code, message, penalty)
    clean_ids = tuple(sorted(set(integers)))
    if valid_ids is None:
        return (clean_ids, penalty)
    valid = tuple(concept_id for concept_id in clean_ids if concept_id in valid_ids)
    invalid_count = len(clean_ids) - len(valid)
    if invalid_count:
        invalid_penalty = MALFORMED_CONFIDENCE_PENALTY * invalid_count
        _append_parse_warning(
            warnings,
            entry_path,
            "invalid_crystal_concept_ids",
            "ignored unknown or inactive crystal concept ids",
            invalid_penalty,
        )
        penalty += invalid_penalty
    return (valid, penalty)


def _concept_names_from_payload(
    payload: dict[object, object],
    entry_path: str,
    warnings: list[DreamParseWarning],
) -> tuple[tuple[str, ...], float]:
    values: list[object] = []
    for key in ("concept_names", "concepts"):
        value = payload.get(key)
        if type(value) is list:
            values.extend(value)
        elif value is not None:
            values.append(value)
    name = payload.get("concept_name")
    if name is not None:
        values.append(name)

    names: list[str] = []
    penalty = 0.0
    for value in values:
        if isinstance(value, str):
            if value.strip():
                names.append(value)
            else:
                penalty += MALFORMED_CONFIDENCE_PENALTY
        elif type(value) is dict:
            name, name_penalty = _recover_optional_concept_name(value)
            penalty += name_penalty
            if name:
                names.append(name)
        else:
            penalty += MALFORMED_CONFIDENCE_PENALTY
    if penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_crystal_concept_metadata",
            "ignored malformed crystal concept metadata",
            penalty,
        )
    return (_clean_text_tuple(tuple(names)), penalty)


def _recover_optional_concept_name(payload: dict[object, object]) -> tuple[str, float]:
    name = _string_field(payload.get("canonical_name")).strip()
    if name:
        return (name, 0.0)
    name = _string_field(payload.get("name")).strip()
    if name:
        return (name, 0.0)
    name = _string_field(payload.get("label")).strip()
    if name:
        return (name, MALFORMED_CONFIDENCE_PENALTY)
    return ("", MALFORMED_CONFIDENCE_PENALTY)


def _clean_string_tuple(*values: object) -> tuple[str, ...]:
    strings: list[str] = []
    for value in values:
        if isinstance(value, str):
            strings.append(value)
        elif type(value) is list:
            strings.extend(item for item in value if isinstance(item, str))
    return _clean_text_tuple(tuple(strings))


def _clean_int_tuple(*values: object) -> tuple[int, ...]:
    integers: list[int] = []
    for value in values:
        if isinstance(value, bool):
            continue
        if isinstance(value, int):
            integers.append(value)
        elif type(value) is list:
            integers.extend(
                item for item in value if isinstance(item, int) and not isinstance(item, bool)
            )
    return tuple(sorted(set(integers)))


def _clean_text_tuple(values: tuple[str, ...]) -> tuple[str, ...]:
    return tuple(sorted(normalize_string_tuple(values)))


def _title_from_kind(kind: str) -> str:
    words = re.findall(r"[A-Za-z0-9А-Яа-яЁё]+", kind.replace("-", " "))
    if not words:
        return ""
    return " ".join(word[:1].upper() + word[1:] for word in words[:4])


def _crystal_type_for_short_memory(memory: ShortTermMemoryRecord) -> str | None:
    if memory.source_credibility == "user_rule" or memory.rule_intent.strip():
        return "rule"
    return "concept"


def _confidence_for_short_memory(memory: ShortTermMemoryRecord) -> float:
    return SOURCE_CREDIBILITY_CONFIDENCE.get(
        memory.source_credibility,
        SOURCE_CREDIBILITY_CONFIDENCE["observation"],
    )
