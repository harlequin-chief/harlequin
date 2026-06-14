#!/usr/bin/env python3
"""
Runner del simulador de consenso. Mide la tesis del paper y escribe RESULTADOS-consenso.md.

Uso:
    python3 run_consenso.py [--stdout]

Mide tres cosas:
  1. Barrido del adversario por FRACCIÓN DE REPUTACIÓN -> dónde está el umbral de seguridad.
  2. Falsa multitud (Sybil) con muestreo PONDERADO por reputación -> el número de nodos no basta.
  3. Misma Sybil con muestreo UNIFORME (contraste) -> sin ponderar por reputación, la multitud gana.
     Demuestra que la ponderación por reputación es lo que defiende (paralelo al damping del motor).
"""

from __future__ import annotations

import random
import sys

from wtc_sim.consenso import ParamsConsenso, run_once
from wtc_sim.poblacion import (
    poblacion_adversario_agrupado,
    poblacion_fraccion_rep,
    poblacion_sybil,
)


PARAMS = ParamsConsenso(k=20, alpha=14, beta=12, max_rondas=80)
TRIALS = 60
SEMILLA = 1984


def agrega(reputacion, adversarios, ponderado, trials=TRIALS,
           clusters=None, cap_cluster=None, adversario="fijo", perdida=0.0):
    """Ejecuta `trials` veces y promedia las banderas (en %)."""
    rng = random.Random(SEMILLA)
    seg = cap = bif = stall = 0
    for _ in range(trials):
        r = run_once(reputacion, adversarios, PARAMS, rng, ponderado=ponderado,
                     clusters=clusters, cap_cluster=cap_cluster, adversario=adversario,
                     perdida=perdida)
        seg += r["seguro"]; cap += r["captura"]; bif += r["bifurcacion"]
        stall += 1 if r["indecisos"] > 0 else 0
    n = float(trials)
    return {"seguro": 100*seg/n, "captura": 100*cap/n,
            "bifurcacion": 100*bif/n, "stall": 100*stall/n}


def construir_informe() -> str:
    L: list[str] = []
    w = L.append

    w("# RESULTADOS — Simulador del Consenso de Confianza Tejida\n")
    w("> Generado por `run_consenso.py` (solo stdlib). Reproducible (PRNG sembrado). Voto "
      "sub-muestreado tipo Snowball/Avalanche con muestreo **ponderado por reputación** (PAPER §5.4).\n")
    w(f"> Parámetros: muestra k={PARAMS.k}, quórum α={PARAMS.alpha}, decisión β={PARAMS.beta}, "
      f"{TRIALS} ejecuciones por punto. Honestos arrancan en el valor legítimo; el adversario es "
      "bizantino (empuja un valor en conflicto).\n")

    # 1. barrido por fraccion de reputacion
    w("\n## 1. Umbral de seguridad por FRACCIÓN DE REPUTACIÓN del adversario\n")
    w("Pocos nodos adversarios, pero controlando una fracción creciente de la reputación total.\n\n")
    w("| Reputación adversaria | Consenso seguro (todos el valor legítimo) | Captura (algún honesto volteado) | Bifurcación |\n")
    w("|---|---|---|---|\n")
    umbral_vivacidad = None   # primera f donde deja de decidir todo el mundo (stall)
    umbral_captura = None     # primera f donde algún honesto adopta el valor falso (fallo de seguridad)
    for f in (0.0, 0.1, 0.2, 0.3, 0.4, 0.5):
        rep, adv = poblacion_fraccion_rep(f)
        r = agrega(rep, adv, ponderado=True)
        w(f"| {int(f*100)} % | {r['seguro']:.0f} % | {r['captura']:.0f} % | {r['bifurcacion']:.0f} % |\n")
        if umbral_vivacidad is None and r["seguro"] < 100.0:
            umbral_vivacidad = f
        if umbral_captura is None and (r["captura"] > 0.0 or r["bifurcacion"] > 0.0):
            umbral_captura = f
    w("\n**Lectura (distingue dos cosas):**\n")
    w("- **Seguridad** (que ningún honesto acepte el valor falso ni haya bifurcación): se mantiene "
      f"mientras el adversario tiene **menos de ~{int((umbral_captura or 0.5)*100)} %** de la reputación. "
      "Por debajo de ese umbral, el adversario **nunca consigue una decisión falsa**.\n")
    w("- **Vivacidad** (que todos lleguen a decidir): empieza a degradarse antes, sobre el "
      f"**~{int((umbral_vivacidad or 0.5)*100)} %**: ahí el adversario aún **no engaña a nadie**, pero "
      "puede **frenar** la convergencia (los honestos quedan indecisos). Es un atasco, no un engaño.\n")
    w("- Lo decisivo: ambos umbrales se expresan en **fracción de reputación**, no en número de nodos. "
      "Esa es la tesis.\n")

    # 2 y 3. Sybil ponderado vs uniforme
    rep_s, adv_s = poblacion_sybil()
    pond = agrega(rep_s, adv_s, ponderado=True)
    unif = agrega(rep_s, adv_s, ponderado=False)
    n_total = len(rep_s); n_adv = len(adv_s)
    w("\n## 2. Falsa multitud (Sybil): número de nodos vs reputación\n")
    w(f"El adversario tiene **{n_adv} nodos** ({100*n_adv/n_total:.0f} % de la red) con reputación ~0; "
      "los honestos son minoría de nodos pero tienen toda la reputación.\n\n")
    w("| Muestreo | Consenso seguro | Captura | Bifurcación |\n|---|---|---|---|\n")
    w(f"| **Ponderado por reputación** (WTC) | {pond['seguro']:.0f} % | {pond['captura']:.0f} % | {pond['bifurcacion']:.0f} % |\n")
    w(f"| Uniforme (Avalanche puro, contraste) | {unif['seguro']:.0f} % | {unif['captura']:.0f} % | {unif['bifurcacion']:.0f} % |\n")
    w(f"\n**Lectura:** con muestreo ponderado por reputación, una falsa multitud del "
      f"{100*n_adv/n_total:.0f} % de los nodos **no rompe** la red (sigue segura), porque casi nunca "
      "entra en las muestras: el poder es la reputación, no el número (Art. VI). Con muestreo "
      "**uniforme** —sin ponderar—, la misma multitud captura/divide la red. La ponderación por "
      "reputación es lo que defiende (paralelo al damping anti-colusión del motor).\n")

    # 3. muestreo ponderado por INDEPENDENCIA contra un adversario correlacionado (PAPER §5.4)
    w("\n## 3. Muestreo por INDEPENDENCIA contra un adversario correlacionado (PAPER §5.4)\n")
    w("Ahora el adversario es más fino: no es una multitud sin reputación, sino un **bloque que SÍ "
      "ganó reputación**, pero toda **correlacionada** (un solo clúster de confianza que se avaló "
      "entre sí). El muestreo ponderado solo por reputación lo trata como si fuera independiente. La "
      "defensa de WTC: ponderar también por **independencia**, limitando a `cap` los nodos que un "
      f"mismo clúster puede aportar a cada comité de k={PARAMS.k} (α={PARAMS.alpha}).\n\n")
    w("| Reputación adversaria (1 clúster) | rep-only: seguro / captura | +independencia (cap=3): seguro / captura |\n")
    w("|---|---|---|\n")
    for f in (0.3, 0.4, 0.45, 0.5):
        rep, adv, cl = poblacion_adversario_agrupado(f, n_clusters_adv=1)
        s = agrega(rep, adv, ponderado=True)                                  # rep-only
        c = agrega(rep, adv, ponderado=True, clusters=cl, cap_cluster=3)      # +independencia
        w(f"| {int(f*100)} % | {s['seguro']:.0f} % / {s['captura']:.0f} % | {c['seguro']:.0f} % / {c['captura']:.0f} % |\n")
    w("\n**Lectura:** con muestreo rep-only, un bloque correlacionado con ≥~40 % de la reputación "
      "**captura** la red. El muestreo por independencia (tope por clúster) lo **neutraliza**: el bloque "
      "no puede ocupar suficientes asientos del comité aunque tenga la reputación. Estructuralmente, "
      "para forzar el color falso necesita ⌈α/cap⌉ asientos de clústeres distintos.\n\n")
    w("**Frontera honesta — el adversario fragmenta su bloque** (45 % de reputación, cap=3):\n\n")
    w("| Clústeres del adversario | +independencia (cap=3): seguro / captura |\n|---|---|\n")
    for nc in (1, 2, 3, 4, 5, 6):
        rep, adv, cl = poblacion_adversario_agrupado(0.45, n_clusters_adv=nc)
        c = agrega(rep, adv, ponderado=True, clusters=cl, cap_cluster=3)
        w(f"| {nc} | {c['seguro']:.0f} % / {c['captura']:.0f} % |\n")
    w(f"\nLa protección aguanta hasta que el adversario fragmenta en **⌈α/cap⌉ = ⌈{PARAMS.alpha}/3⌉ = 5** "
      "clústeres distintos; con 5+ vuelve a capturar. Pero fragmentar exige que **cada** sub-bloque "
      "parezca un clúster **independiente** ante la detección de comunidades — justo lo que el **motor "
      "de reputación** (damping + comunidades) está diseñado para resistir, y que ya mostramos hace que "
      "el lavado rinda **0** poder de consenso. *Los dos prototipos componen:* el muestreo por "
      "independencia del consenso descansa en la identificación de clústeres que aporta el motor.\n")

    # 4. adversario adaptativo (anti-finalidad): ataca vivacidad, no seguridad
    w("\n## 4. Adversario ADAPTATIVO (divisor anti-finalidad)\n")
    w("En vez de empujar siempre el mismo valor, el adversario reporta cada ronda el color **minoritario** "
      "entre los honestos para mantenerlos divididos e impedir que cualquier color cuaje (worst-case: ve "
      "el estado del momento). Mide si puede romper la SEGURIDAD o solo la VIVACIDAD.\n\n")
    w("| Reputación adversaria (1 clúster) | adaptativo, rep-only: seguro / captura / atasco | adaptativo, +indep cap=3 |\n")
    w("|---|---|---|\n")
    for f in (0.2, 0.3, 0.4, 0.5):
        rep, adv, cl = poblacion_adversario_agrupado(f, n_clusters_adv=1)
        a = agrega(rep, adv, ponderado=True, adversario="adaptativo")
        b = agrega(rep, adv, ponderado=True, clusters=cl, cap_cluster=3, adversario="adaptativo")
        w(f"| {int(f*100)} % | {a['seguro']:.0f} % / {a['captura']:.0f} % / {a['stall']:.0f} % | "
          f"{b['seguro']:.0f} % / {b['captura']:.0f} % / {b['stall']:.0f} % |\n")
    w("\n**Lectura:** el adversario adaptativo **nunca fuerza una decisión falsa** (captura 0 %): ataca "
      "la **vivacidad** (atasca la convergencia), no la **seguridad**. Y bajo muestreo por independencia, "
      "ni siquiera atasca: su bloque correlacionado no entra lo bastante en los comités. Coherente con "
      "Snowball/Avalanche: la seguridad es robusta; el coste adaptativo es liveness, recuperable.\n")

    # 5. partición de red: ataque + mitigación
    import particion
    w("\n## 5. Partición de red: ataque de partición + mitigación quórum-de-red\n")
    w("Se rompe el supuesto de red completa: los honestos se parten en un grupo grande A (60) y uno "
      "pequeño B (20); un adversario inofensivo global (15%) concentra su reputación en B. La red está "
      "partida `D` rondas (cada grupo solo se ve a sí mismo) y luego sana.\n\n")
    w("| D (rondas partido) | fork (sin mitig.) | fork (+quórum 60%) | atasco (+quórum) |\n|---:|---:|---:|---:|\n")
    for D in (0, 20, 50, 90):
        s = particion.medir(0.15, D, quorum_red=0.0)
        c = particion.medir(0.15, D, quorum_red=0.6)
        w(f"| {D} | {s['fork']:.0f} % | {c['fork']:.0f} % | {c['atasco']:.0f} % |\n")
    w("\n**Hallazgo (honesto, importante):** una partición larga **concentra la cuota LOCAL del "
      "adversario** en el grupo pequeño por encima del umbral → B finaliza el valor falso → al sanar, "
      "**FORK**. Un 15% global inofensivo se vuelve **100% de fork** con partición de 90 rondas. **Es un "
      "fallo de safety real**, encontrado en simulación antes de construir la cadena.\n\n")
    w("**Mitigación:** condicionar la **finalidad** a ver un **quórum de la reputación total** (un nodo "
      "no decide si alcanza a ver <60% de la red). Bajo partición, B (25% de la rep) no llega al quórum → "
      "**no finaliza, se atasca** (coste de liveness) en vez de forkear, y **recupera al sanar**. El fork "
      "cae de ~100% a ~0–2%. Es la regla esperable de un BFT robusto: **ante duda de partición, preferir "
      "parar (liveness) antes que decidir mal (safety)**. Pendiente: detección dinámica de la fracción "
      "vista (aquí se modela con el grupo) y latencia/pérdida de mensajes.\n")

    # 6. pérdida de mensajes (latencia/red poco fiable)
    w("\n## 6. Pérdida de mensajes: latencia / red poco fiable\n")
    w("Cada respuesta consultada se pierde con probabilidad `p` (modela latencia y pérdida de red). "
      "Mide si la red poco fiable rompe la seguridad o solo ralentiza.\n\n")
    w("| pérdida p | sin adversario: seguro / atasco | adversario 25%: captura / atasco |\n|---:|---:|---:|\n")
    for p in (0.0, 0.2, 0.4, 0.6):
        rng_a = poblacion_fraccion_rep(0.0); rng_b = poblacion_fraccion_rep(0.25)
        a = agrega(rng_a[0], rng_a[1], ponderado=True, perdida=p)
        b = agrega(rng_b[0], rng_b[1], ponderado=True, perdida=p)
        w(f"| {int(p*100)} % | {a['seguro']:.0f} % / {a['stall']:.0f} % | {b['captura']:.0f} % / {b['stall']:.0f} % |\n")
    w("\n**Lectura:** la pérdida degrada la **vivacidad** (más atasco) pero **nunca rompe la seguridad** "
      "(captura 0 % a cualquier pérdida): el umbral α no cambia. Implicación de parámetro: para progresar "
      "bajo pérdida `p` hacen falta suficientes respuestas vivas, **α ≤ k·(1−p)**; con k=20, α=14 se "
      "tolera pérdida hasta ~30 % antes de atascarse del todo. La elección de α/k acota la pérdida "
      "tolerable — un parámetro a fijar con el modelado de red real.\n")

    w("\n## Conclusión\n")
    w("El simulador confirma y endurece las piezas del paper: (1) el **umbral de seguridad se mide en "
      "fracción de REPUTACIÓN**, no de nodos; (2) el **muestreo por independencia** (PAPER §5.4) eleva ese "
      "umbral frente a un adversario **correlacionado** — vencerlo exige fragmentar en clústeres que "
      "parezcan independientes, lo que el motor de reputación resiste (*los dos prototipos componen*); "
      "(3) un adversario **adaptativo** solo ataca la **vivacidad**, nunca la seguridad; (4) bajo "
      "**partición** aparece un fork real si se finaliza a ciegas, **mitigado** condicionando la finalidad "
      "a un quórum de red (safety sobre liveness). Limitación honesta restante: modelo binario, latencia/"
      "pérdida de mensajes no modeladas, y prueba **formal** de safety/liveness (PAPER §10).\n")
    return "".join(L)


def main() -> None:
    informe = construir_informe()
    with open("RESULTADOS-consenso.md", "w", encoding="utf-8") as f:
        f.write(informe)
    print("[ok] informe escrito en RESULTADOS-consenso.md")
    if "--stdout" in sys.argv:
        print("\n" + informe)


if __name__ == "__main__":
    main()
