"""
Modelo de datos: dimensiones de reputación y agentes (seudónimos).

Ancla en SPEC.md:
- §1.2b: reputación VECTORIAL. Dimensiones iniciales (extensibles): comercio, contribución
  técnica, función judicial, gobernanza. Ganar en una NO contamina las otras.
- §1.4: DOS PUERTAS. Puerta 1 = personalidad (humano único) -> ciudadanía base = 1, sin padrino.
  Puerta 2 = reputación ganada por encima de la base.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum


# §1.2b — dimensiones iniciales de la reputación vectorial (conjunto extensible).
DIMENSIONES: tuple[str, ...] = (
    "comercio",
    "contribucion_tecnica",
    "funcion_judicial",
    "gobernanza",
)


class TipoAgente(str, Enum):
    """Etiqueta SOLO para análisis de la simulación (el protocolo no la conoce)."""

    GENESIS = "genesis"        # cohorte fundadora (semilla, §1.4); diseñada para diluirse
    HONESTO = "honesto"        # miembro real con historial verificable
    SYBIL = "sybil"            # identidad falsa sin trabajo real (§1.5)
    COLUSOR = "colusor"        # parte de un anillo de avales mutuos (§1.6)
    BLANQUEADOR = "blanqueador"  # abandona un seudónimo "quemado" y reinicia (whitewashing)


@dataclass
class Agente:
    """
    Un seudónimo de la red. La identidad física NUNCA se modela: solo el seudónimo (Art. VII).

    - es_humano_unico: pasó la prueba de personalidad (§1.5). Da ciudadanía base = 1.
    - evidencia: trabajo/tratos VALIDADOS por dimensión (§1.3a). Es el ancla "real" de la
      reputación: sin evidencia objetiva, los avales no pueden, por sí solos, fabricar poder.
    - el vector de reputación ganada NO se guarda aquí: se DERIVA del grafo + evidencia
      (ver reputacion.reputacion_vectorial). Aquí solo viven las entradas del modelo.
    """

    id: str
    tipo: TipoAgente
    es_humano_unico: bool = True
    # evidencia objetiva verificable por dimensión (tratos liquidados, trabajo comprobado).
    evidencia: dict[str, float] = field(default_factory=dict)
    cluster: str | None = None  # etiqueta de anillo, solo para construir/analizar escenarios

    @property
    def base(self) -> float:
        """Ciudadanía base (§1.4): 1 si es persona única verificada, 0 si no."""
        return 1.0 if self.es_humano_unico else 0.0

    def evidencia_dim(self, dim: str) -> float:
        return self.evidencia.get(dim, 0.0)
