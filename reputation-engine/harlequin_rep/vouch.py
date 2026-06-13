"""
Mecánica de avales: cupo, responsabilidad persistente, dividendo de mentor y slashing en cascada.

Ancla en SPEC.md:
- §1.5c: incentivo a apadrinar SOLO reputacional; vínculo padrino->ahijado PERSISTENTE.
  - cara negativa: si el ahijado defrauda, el padrino PIERDE reputación (aunque pase el tiempo).
  - cara positiva: dividendo de mentor = eco pequeño de la reputación INDEPENDIENTE del ahijado;
    apadrinar títeres (reputación dependiente del propio cluster) NO rinde.
  - cupo de avales vivos = función SUBLINEAL de la reputación (rendimientos decrecientes).
- §1.7: slashing por fraude probado.

Esto NO recalcula el EigenTrust; opera sobre un vector de reputación ya calculado para mostrar las
DINÁMICAS de incentivos (qué le pasa al padrino cuando el ahijado prospera o defrauda).
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field


@dataclass
class Apadrinamiento:
    padrino: str
    ahijado: str
    vivo: bool = True  # se libera al "graduarse" el ahijado, pero la responsabilidad no caduca


@dataclass
class RegistroAvales:
    """Lleva la cuenta de quién avaló a quién (responsabilidad persistente, §1.5c)."""

    vinculos: list[Apadrinamiento] = field(default_factory=list)

    def apadrinar(self, padrino: str, ahijado: str) -> None:
        self.vinculos.append(Apadrinamiento(padrino, ahijado))

    def padrinos_de(self, ahijado: str) -> list[str]:
        return [v.padrino for v in self.vinculos if v.ahijado == ahijado]

    def avales_vivos(self, padrino: str) -> int:
        return sum(1 for v in self.vinculos if v.padrino == padrino and v.vivo)


def cupo_de_avales(reputacion_agregada: float, k: float = 3.0) -> int:
    """
    Cupo de avales VIVOS = función sublineal de la reputación (§1.5c): más reputación -> más cupo,
    con rendimientos decrecientes para que nadie monopolice el apadrinamiento.

    cupo = floor(k * log2(1 + rep))   (sublineal, crece despacio).
    """
    return int(k * math.log2(1.0 + max(0.0, reputacion_agregada)))


def dividendo_de_mentor(
    rep_independiente_ahijado: float,
    eco: float = 0.05,
) -> float:
    """
    Dividendo de mentor (§1.5c, cara positiva): el padrino gana un eco PEQUEÑO de la reputación
    INDEPENDIENTE del ahijado (ya descontada por cercanía en el grafo, §1.6). Apadrinar a tus
    propios títeres -> su reputación independiente ~ 0 -> dividendo ~ 0 -> no rinde.
    """
    return eco * max(0.0, rep_independiente_ahijado)


def slashing_en_cascada(
    reputacion: dict[str, float],
    registro: RegistroAvales,
    culpable: str,
    perdida: float,
    fraccion_padrino: float = 0.5,
    profundidad: int = 3,
) -> dict[str, float]:
    """
    Slashing por fraude probado (§1.7) con RESPONSABILIDAD PERSISTENTE en cascada (§1.5c).

    El culpable pierde `perdida`. Cada padrino pierde una FRACCIÓN de lo que perdió su ahijado
    (responsabilidad por a quién metió), y así hacia arriba en la cadena de avales hasta
    `profundidad`. Devuelve un NUEVO dict de reputación (no muta el de entrada).

    Esto es lo que hace cara la colusión: montar un anillo y que uno defraude arrastra a sus
    avaladores. Apadrinar a la ligera sale caro.
    """
    nueva = dict(reputacion)

    def aplicar(agente: str, monto: float, nivel: int) -> None:
        if monto <= 0 or nivel < 0 or agente not in nueva:
            return
        nueva[agente] = max(0.0, nueva[agente] - monto)
        repercute = monto * fraccion_padrino
        for padrino in registro.padrinos_de(agente):
            aplicar(padrino, repercute, nivel - 1)

    aplicar(culpable, perdida, profundidad)
    return nueva
