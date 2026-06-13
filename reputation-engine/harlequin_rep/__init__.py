"""
harlequin_rep — Prototipo del motor de reputación de Harlequin.

Implementa, en código ejecutable, el núcleo descrito en SPEC.md §1 (reputación) y §2.2
(consenso por sorteo ponderado por reputación). Solo biblioteca estándar de Python: sin
dependencias externas (criterio OPSEC del proyecto).

Objetivo: dejar de tener el diseño solo en papel y poder MEDIR si las defensas anti-Sybil y
anti-colusión (el frente de riesgo #1 de la SPEC, §1.6) hacen lo que prometen.

Mapa SPEC -> código:
- §1.2b reputación vectorial (contextual)      -> model.Dimension, reputacion.reputacion_vectorial
- §1.3  cómo se gana (evidencia + atestación)  -> graph.GrafoConfianza, reputacion (anclaje pretrust)
- §1.4  dos puertas (ciudadanía base vs ganada)-> model.Agente.base / .es_humano_unico
- §1.6  anti-colusión (damping por cluster)    -> graph.independencia + reputacion (EigenTrust amortiguado)
- §1.7  decaimiento y slashing                 -> vouch.slashing_en_cascada, reputacion.decaer
- §2.2  consenso: sorteo ponderado por rep.    -> consenso.sorteo_ponderado
"""

from .model import Agente, TipoAgente, DIMENSIONES
from .graph import GrafoConfianza
from .reputacion import reputacion_vectorial, agregado_conservador
from .consenso import sorteo_ponderado

__all__ = [
    "Agente",
    "TipoAgente",
    "DIMENSIONES",
    "GrafoConfianza",
    "reputacion_vectorial",
    "agregado_conservador",
    "sorteo_ponderado",
]
