"""
Escenarios de red para las simulaciones de ataque.

Cada escenario construye una población de seudónimos + grafo de avales y etiqueta las facciones
para poder medir cuánto poder captura cada tipo de actor. Todo con un PRNG sembrado -> reproducible.

Escenarios:
  1. honesto      — línea base: génesis + miembros honestos con evidencia real e independiente.
  2. sybil        — base + 200 identidades falsas (personhood "rota", peor caso) sin evidencia.
  3. colusion     — base + anillo de 30 que se avalan en círculo para granjear reputación.
  4. blanqueo     — un seudónimo consolidado vs uno nuevo (whitewashing): qué se pierde al reiniciar.
"""

from __future__ import annotations

import random
from dataclasses import dataclass

from harlequin_rep.graph import GrafoConfianza
from harlequin_rep.model import DIMENSIONES, Agente, TipoAgente


@dataclass
class Escenario:
    nombre: str
    descripcion: str
    agentes: list[Agente]
    grafo: GrafoConfianza
    facciones: dict[str, str]  # id -> facción ("genesis"/"honesto"/"sybil"/"colusor"/"blanqueador")


def _construir_base(rng: random.Random) -> tuple[list[Agente], GrafoConfianza, dict[str, str]]:
    """
    Red honesta base: 5 génesis + 40 honestos.

    - Génesis: evidencia moderada en todas las dimensiones (semilla, §1.4) y avalan a quienes vetan.
    - Honestos: evidencia real en 1-2 dimensiones (tratos liquidados, §1.3a). Reciben avales de
      varias contrapartes INDEPENDIENTES (poco solapamiento -> alta independencia, §1.6).
    """
    agentes: list[Agente] = []
    facciones: dict[str, str] = {}
    grafo = GrafoConfianza()

    genesis = [f"g{i}" for i in range(5)]
    for gid in genesis:
        a = Agente(
            id=gid,
            tipo=TipoAgente.GENESIS,
            es_humano_unico=True,
            evidencia={d: 2.0 for d in DIMENSIONES},
        )
        agentes.append(a)
        facciones[gid] = "genesis"

    honestos = [f"h{i}" for i in range(40)]
    for hid in honestos:
        ndims = rng.randint(2, 3)
        dims = rng.sample(DIMENSIONES, ndims)
        evidencia = {d: rng.uniform(1.0, 5.0) for d in dims}
        a = Agente(id=hid, tipo=TipoAgente.HONESTO, es_humano_unico=True, evidencia=evidencia)
        agentes.append(a)
        facciones[hid] = "honesto"

    # génesis avalan a un puñado de honestos que "vetaron" (apadrinamiento de cohorte, §1.4)
    establecidos = genesis + honestos
    for gid in genesis:
        for hid in rng.sample(honestos, 4):
            for d in DIMENSIONES:
                grafo.atestar(gid, hid, d, 1.0)

    # cada honesto recibe avales de 2-4 contrapartes INDEPENDIENTES en su(s) dimensión(es) de
    # evidencia (quien hizo el trato con él lo atesta, §1.3a). Avaladores distintos -> poco
    # solapamiento -> independencia alta (§1.6).
    for a in agentes:
        if a.tipo is not TipoAgente.HONESTO:
            continue
        for d in a.evidencia:
            k = rng.randint(2, 4)
            avaladores = rng.sample([e for e in establecidos if e != a.id], k)
            for av in avaladores:
                grafo.atestar(av, a.id, d, 1.0)

    return agentes, grafo, facciones


def escenario_honesto(semilla: int = 7) -> Escenario:
    rng = random.Random(semilla)
    agentes, grafo, facciones = _construir_base(rng)
    return Escenario(
        nombre="honesto",
        descripcion="Línea base: génesis + 40 honestos con evidencia real y avales independientes.",
        agentes=agentes,
        grafo=grafo,
        facciones=facciones,
    )


def escenario_sybil(semilla: int = 7, n_sybil: int = 200) -> Escenario:
    """
    Peor caso de Sybil: las 200 cuentas falsas SÍ pasan la prueba de personalidad (es_humano_unico
    True) — asumimos que la IA reventó la puerta base (§1.5). Pero NO tienen evidencia real ni avales
    de reputados; solo se avalan un poco entre ellas. Tesis (§1.5, §2.4): poder ~ 0.
    """
    rng = random.Random(semilla)
    agentes, grafo, facciones = _construir_base(rng)

    sybils = [f"s{i}" for i in range(n_sybil)]
    for sid in sybils:
        agentes.append(Agente(id=sid, tipo=TipoAgente.SYBIL, es_humano_unico=True, evidencia={}))
        facciones[sid] = "sybil"

    # se avalan entre ellas al azar (sin coste real), pero ningún reputado las avala
    for sid in sybils:
        for otro in rng.sample(sybils, 3):
            if otro != sid:
                grafo.atestar(sid, otro, "comercio", 1.0)

    return Escenario(
        nombre="sybil",
        descripcion=f"Base + {n_sybil} identidades falsas (personhood rota, peor caso) sin evidencia.",
        agentes=agentes,
        grafo=grafo,
        facciones=facciones,
    )


def escenario_colusion(
    semilla: int = 7, n_anillo: int = 30, honestos_engañados: int = 3
) -> Escenario:
    """
    Anillo de colusión (§1.6): 30 cuentas que se avalan en círculo (clique reciprocal denso) para
    granjear reputación, sin trabajo real. Para ponerlo DURO, 3 honestos "engañados" avalan cada uno
    a un miembro del anillo (inyección de confianza real). Sin damping, el anillo recircularía esa
    inyección y se inflaría; con damping (independencia ~0 en aristas mutuas) no puede.
    """
    rng = random.Random(semilla)
    agentes, grafo, facciones = _construir_base(rng)

    anillo = [f"c{i}" for i in range(n_anillo)]
    for idx, cid in enumerate(anillo):
        # c0 SÍ hizo trabajo real (tiene evidencia): es el ancla que el anillo intentará
        # amplificar y repartir entre los otros 29 vía avales mutuos. El damping (§1.6) debe
        # impedir esa propagación: c0 conserva SU reputación legítima, el resto se queda en ~0.
        evidencia = {"comercio": 20.0} if idx == 0 else {}
        agentes.append(Agente(id=cid, tipo=TipoAgente.COLUSOR, es_humano_unico=True,
                              evidencia=evidencia, cluster="anillo1"))
        facciones[cid] = "colusor"

    # clique reciprocal: cada colusor avala a todos los demás del anillo en comercio (granja)
    for a in anillo:
        for b in anillo:
            if a != b:
                grafo.atestar(a, b, "comercio", 1.0)

    # inyección: 3 honestos engañados avalan a 3 miembros del anillo
    honestos = [aid for aid, f in facciones.items() if f == "honesto"]
    for hid, cid in zip(rng.sample(honestos, honestos_engañados), anillo):
        grafo.atestar(hid, cid, "comercio", 1.0)

    return Escenario(
        nombre="colusion",
        descripcion=f"Base + anillo de {n_anillo} avalándose en círculo + {honestos_engañados} "
        f"honestos engañados que lo avalan.",
        agentes=agentes,
        grafo=grafo,
        facciones=facciones,
    )


def escenario_colusion_dispersa(
    semilla: int = 7, n_anillo: int = 30, grado: int = 3, honestos_engañados: int = 3
) -> Escenario:
    """
    Colusión SOFISTICADA (frente abierto §1.6): en vez de un clique denso (todos avalan a todos),
    un anillo DISPERSO donde cada colusor avala solo a `grado` otros del anillo, elegidos para
    minimizar la reciprocidad y el solapamiento de vecinos (imita patrones honestos). Es el ataque
    que más cuesta detectar: pocas firmas obvias de endogamia. c0 tiene reputación real que el
    anillo intenta lavar. Sirve para MEDIR si el damping (pensado contra cliques) también frena al
    anillo disperso, o se filtra (revelando la frontera).
    """
    rng = random.Random(semilla)
    agentes, grafo, facciones = _construir_base(rng)

    anillo = [f"c{i}" for i in range(n_anillo)]
    for idx, cid in enumerate(anillo):
        evidencia = {"comercio": 20.0} if idx == 0 else {}
        agentes.append(Agente(id=cid, tipo=TipoAgente.COLUSOR, es_humano_unico=True,
                              evidencia=evidencia, cluster="anillo_disperso"))
        facciones[cid] = "colusor"

    # anillo disperso: cada colusor avala a `grado` otros, evitando reciprocidad directa cuando se
    # puede (baja la firma de endogamia). Topología tipo grafo aleatorio dirigido disperso.
    for a in anillo:
        candidatos = [b for b in anillo if b != a]
        for b in rng.sample(candidatos, min(grado, len(candidatos))):
            grafo.atestar(a, b, "comercio", 1.0)

    honestos = [aid for aid, f in facciones.items() if f == "honesto"]
    for hid, cid in zip(rng.sample(honestos, honestos_engañados), anillo):
        grafo.atestar(hid, cid, "comercio", 1.0)

    return Escenario(
        nombre="colusion_dispersa",
        descripcion=f"Anillo DISPERSO de {n_anillo} (grado {grado}, baja endogamia) + {honestos_engañados} "
        f"honestos engañados. Colusión sofisticada (frente §1.6).",
        agentes=agentes,
        grafo=grafo,
        facciones=facciones,
    )


def escenario_blanqueo(semilla: int = 7) -> Escenario:
    """
    Whitewashing (§5, §1): un seudónimo CONSOLIDADO (blanq_viejo, con evidencia + avales) frente a
    uno NUEVO (blanq_nuevo, solo ciudadanía base). Demuestra que abandonar el seudónimo para
    "empezar limpio" cuesta TODA la reputación ganada: la reputación se ata a la máscara, no se
    transfiere (Art. VII/VIII).
    """
    rng = random.Random(semilla)
    agentes, grafo, facciones = _construir_base(rng)
    establecidos = [a.id for a in agentes]

    viejo = Agente(id="blanq_viejo", tipo=TipoAgente.BLANQUEADOR, es_humano_unico=True,
                   evidencia={"comercio": 4.0})
    agentes.append(viejo)
    facciones["blanq_viejo"] = "blanqueador"
    for av in rng.sample(establecidos, 4):
        grafo.atestar(av, "blanq_viejo", "comercio", 1.0)

    # mismo humano, seudónimo nuevo: solo ciudadanía base, sin evidencia ni avales heredados
    nuevo = Agente(id="blanq_nuevo", tipo=TipoAgente.BLANQUEADOR, es_humano_unico=True, evidencia={})
    agentes.append(nuevo)
    facciones["blanq_nuevo"] = "blanqueador"

    return Escenario(
        nombre="blanqueo",
        descripcion="Seudónimo consolidado vs seudónimo nuevo del mismo humano (whitewashing).",
        agentes=agentes,
        grafo=grafo,
        facciones=facciones,
    )


TODOS = [escenario_honesto, escenario_sybil, escenario_colusion, escenario_blanqueo]
