#!/usr/bin/env python3
"""
Tests de autoauditoría del motor de reputación. Sin dependencias (no pytest): asserts planos.

Ejecutar:
    python3 tests/test_motor.py     # desde prototipos/reputacion/

Verifica las propiedades que sostienen las conclusiones del informe, para que no sean "parece que
va" sino comprobable.
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from harlequin_rep.graph import GrafoConfianza
from harlequin_rep.model import DIMENSIONES
from harlequin_rep.reputacion import (
    agregado_conservador,
    reputacion_dimension,
    reputacion_vectorial,
)
from harlequin_rep.vouch import RegistroAvales, cupo_de_avales, slashing_en_cascada
import escenarios


def _suma(vec):
    return sum(vec.values())


def test_conservacion_de_masa():
    """EigenTrust con fuga al pre-trust debe conservar la masa: cada dimensión suma ~escala."""
    esc = escenarios.escenario_colusion()
    for dim in DIMENSIONES:
        rep = reputacion_dimension(esc.agentes, esc.grafo, dim, escala=1000.0)
        total = sum(rep.values())
        assert abs(total - 1000.0) < 1.0, f"masa no conservada en {dim}: {total}"


def test_sybil_sin_poder():
    """200 sybils sin evidencia ni avales de reputados -> reputación ~ 0."""
    esc = escenarios.escenario_sybil()
    rep = reputacion_vectorial(esc.agentes, esc.grafo)
    sybils = [aid for aid, f in esc.facciones.items() if f == "sybil"]
    cuota_sybil = sum(_suma(rep[s]) for s in sybils)
    total = sum(_suma(v) for v in rep.values())
    assert cuota_sybil / total < 0.001, f"sybils capturan demasiado: {cuota_sybil/total:.4f}"


def test_damping_reduce_lavado():
    """El damping (§1.6) reduce la reputación lavada del trabajador c0 a sus secuaces."""
    esc = escenarios.escenario_colusion()
    con = reputacion_vectorial(esc.agentes, esc.grafo, damping=True)
    sin = reputacion_vectorial(esc.agentes, esc.grafo, damping=False)
    secuaces = [c for c, f in esc.facciones.items() if f == "colusor" and c != "c0"]
    lavado_con = sum(_suma(con[s]) for s in secuaces)
    lavado_sin = sum(_suma(sin[s]) for s in secuaces)
    assert lavado_con < lavado_sin, "el damping no reduce el lavado"
    assert lavado_sin / max(lavado_con, 1e-9) > 3.0, "el damping aporta menos de 3×"


def test_comunidad_reduce_anillo_disperso():
    """La detección de comunidades reduce el lavado del anillo DISPERSO (frente §1.6) sin dañar a los honestos."""
    esc = escenarios.escenario_colusion_dispersa()
    secuaces = [c for c, f in esc.facciones.items() if f == "colusor" and c != "c0"]
    honestos = [a.id for a in esc.agentes if esc.facciones[a.id] in ("honesto", "genesis")]

    def medir(comunidad):
        rep = reputacion_vectorial(esc.agentes, esc.grafo, damping=True, comunidad=comunidad)
        lavado = sum(_suma(rep[s]) for s in secuaces)
        honesto = sum(_suma(rep[h]) for h in honestos)
        return lavado, honesto

    lav_local, hon_local = medir(False)
    lav_comun, hon_comun = medir(True)
    assert lav_comun < lav_local * 0.8, "la defensa de comunidad debe reducir el lavado disperso"
    assert hon_comun > hon_local * 0.9, "la defensa de comunidad NO debe castigar a los honestos"


def test_adaptativa_comunidad_no_empeora():
    """
    Colusión ADAPTATIVA (fragmentar para evadir la etiqueta de comunidad): a cualquier nivel de
    fragmentación, la defensa de comunidad NUNCA lava más que la local sola y SIEMPRE reduce frente
    a no tener damping. Evadir la etiqueta no le compra al atacante más reputación lavada.
    """
    titeres_de = lambda esc: [c for c, f in esc.facciones.items() if f == "colusor" and c != "c0"]
    for k in (1, 3, 6, 10):
        esc = escenarios.escenario_colusion_adaptativa(n_fragmentos=k)
        tit = titeres_de(esc)
        sin = sum(_suma(reputacion_vectorial(esc.agentes, esc.grafo, damping=False)[t]) for t in tit)
        loc = sum(_suma(reputacion_vectorial(esc.agentes, esc.grafo, damping=True)[t]) for t in tit)
        com = sum(_suma(reputacion_vectorial(esc.agentes, esc.grafo, damping=True, comunidad=True)[t]) for t in tit)
        assert com <= loc + 1e-6, f"k={k}: comunidad ({com:.1f}) lava más que local ({loc:.1f})"
        assert com < sin, f"k={k}: la defensa no reduce frente a sin damping"


def test_adaptativa_sin_poder_de_consenso():
    """
    El lavado adaptativo es UNIDIMENSIONAL (solo 'comercio'): bajo el agregado conservador (min,
    §1.2b) que rige el poder de consenso/aval, los títeres colapsan a ~0 a cualquier fragmentación.
    Filtrar reputación difusa en una dimensión NO compra poder que exige fiabilidad global.
    """
    for k in (1, 3, 10):
        esc = escenarios.escenario_colusion_adaptativa(n_fragmentos=k)
        tit = [c for c, f in esc.facciones.items() if f == "colusor" and c != "c0"]
        rep = reputacion_vectorial(esc.agentes, esc.grafo, damping=True, comunidad=True)
        poder = sum(agregado_conservador(rep[t], "min") for t in tit)
        assert poder < 0.01, f"k={k}: títeres capturan poder de consenso {poder:.4f} (> 0)"


def test_temporal_decae_inactivos():
    """
    Dinámica temporal (§1.7, Art. VI): quien deja de aportar DECAE; quien sigue aportando se
    sostiene. El pionero de una obra única no conserva el poder (anti-long-range); el activo sí.
    """
    import temporal
    tray = temporal.simular()
    activo = tray["honesto_activo"]
    retirado = tray["honesto_retirado"]
    pionero = tray["pionero_durmiente"]
    # el activo termina por encima de donde empezó (aportar mantiene/crece el poder)
    assert activo[-1] > activo[0], "el honesto activo no debería decaer"
    # el retirado decae claramente desde su pico tras dejar de aportar (anti-atrincheramiento)
    assert retirado[-1] < max(retirado) * 0.8, "el retirado debería decaer tras parar"
    # el pionero durmiente pierde la mayor parte de su poder (anti-long-range)
    assert pionero[-1] < max(pionero) * 0.3, "el pionero durmiente debería perder casi todo"
    # los títeres del anillo que farmea-y-se-sienta se desinflan con el tiempo
    titeres = tray["titeres_anillo"]
    assert titeres[-1] < titeres[1], "los títeres del anillo deberían desinflarse"


def test_graduacion_libera_cupo_del_mentor():
    """
    Graduación de ahijados (§1.5c): el ahijado empieza dependiendo del aval del mentor (rep
    independiente ~0) y, al ganar obra+avales propios, gradúa → su rep independiente domina y el
    cupo del mentor se libera (un aval vivo menos). El andamiaje se diluye.
    """
    import graduacion
    tray = graduacion.simular()
    # arranca dependiente (poca o nula reputación independiente) y termina sosteniéndose solo
    assert tray["independiente"][0] < 0.2 * max(tray["total"][0], 1e-9), "A no debería empezar independiente"
    assert tray["graduado"][-1], "A debería haber graduado dentro del horizonte"
    assert tray["independiente"][-1] > 0.8 * tray["total"][-1], "tras graduar, A se sostiene solo"
    # al graduar, el cupo libre del mentor aumenta (se liberó el aval vivo)
    ep = next(t for t in range(len(tray["graduado"])) if tray["graduado"][t])
    assert tray["cupo_libre_M"][ep] > tray["cupo_libre_M"][ep - 1], "graduar debería liberar cupo del mentor"


def test_blanqueo_pierde_todo():
    """El seudónimo nuevo (whitewashing) no hereda reputación: ~0 frente al consolidado."""
    esc = escenarios.escenario_blanqueo()
    rep = reputacion_vectorial(esc.agentes, esc.grafo)
    viejo, nuevo = _suma(rep["blanq_viejo"]), _suma(rep["blanq_nuevo"])
    assert nuevo < 0.01 * max(viejo, 1e-9), "el seudónimo nuevo conserva demasiada reputación"
    assert viejo > 1.0, "el consolidado debería tener reputación apreciable"


def test_slashing_en_cascada():
    """Slashing sube por la cadena de avales; el ajeno no se ve afectado."""
    reputacion = {"mentor": 200.0, "intermedio": 120.0, "ahijado": 100.0, "ajeno": 150.0}
    reg = RegistroAvales()
    reg.apadrinar("mentor", "intermedio")
    reg.apadrinar("intermedio", "ahijado")
    d = slashing_en_cascada(reputacion, reg, "ahijado", 100.0, fraccion_padrino=0.5)
    assert d["ahijado"] == 0.0
    assert d["intermedio"] == 70.0   # -50
    assert d["mentor"] == 175.0      # -25
    assert d["ajeno"] == 150.0       # intacto


def test_agregado_conservador():
    """min <= mediana; min penaliza la dimensión floja (no se compra integridad con pericia)."""
    vec = {"a": 0.0, "b": 100.0, "c": 100.0, "d": 100.0}
    assert agregado_conservador(vec, "min") == 0.0
    assert agregado_conservador(vec, "mediana") == 100.0


def test_independencia_penaliza_endogamia():
    """Un aval recíproco con vecinos compartidos es menos independiente que uno hacia un extraño."""
    g = GrafoConfianza()
    # anillo: a<->b, ambos avalan a x e y (solapamiento alto)
    for u in ("a", "b"):
        for v in ("x", "y"):
            g.atestar(u, v, "comercio")
    g.atestar("a", "b", "comercio")
    g.atestar("b", "a", "comercio")
    # extraño: p avala a q, sin reciprocidad ni vecinos compartidos
    g.atestar("p", "q", "comercio")
    endogamo = g.independencia("a", "b", "comercio")
    extrano = g.independencia("p", "q", "comercio")
    assert endogamo < extrano, f"endogamo {endogamo} debería ser < extraño {extrano}"
    assert extrano == 1.0


def test_cupo_sublineal():
    """El cupo de avales crece sublinealmente (rendimientos decrecientes)."""
    c10, c100, c1000 = cupo_de_avales(10), cupo_de_avales(100), cupo_de_avales(1000)
    assert c10 < c100 < c1000
    assert (c1000 - c100) < (c100 - c10) * 3  # crecimiento desacelera (log)


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    fallos = 0
    for t in tests:
        try:
            t()
            print(f"  PASS  {t.__name__}")
        except AssertionError as e:
            fallos += 1
            print(f"  FAIL  {t.__name__}: {e}")
    print(f"\n{len(tests) - fallos}/{len(tests)} tests OK")
    sys.exit(1 if fallos else 0)


if __name__ == "__main__":
    main()
