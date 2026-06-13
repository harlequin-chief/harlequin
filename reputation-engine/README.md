# Prototipo del motor de reputación de Harlequin

Primer **código ejecutable** del proyecto. Implementa el núcleo descrito en `SPEC.md §1`
(reputación), `§1.6` (anti-colusión) y `§2.2` (consenso por sorteo ponderado por reputación), y lo
**pone a prueba con simulaciones de ataque**. Solo biblioteca estándar de Python 3 — **sin
dependencias externas** (criterio OPSEC del proyecto).

> Esto NO es la blockchain. Es el banco de pruebas del mecanismo más importante para Chief: la
> reputación. Sirve para **medir** si las defensas anti-Sybil y anti-colusión hacen lo que la SPEC
> promete, antes de construir la cadena alrededor. Capa 2 del `ROADMAP.md`, paso 0.

## Qué demuestra

La apuesta central de la SPEC (§1.5, §2.4): **el muro anti-Sybil real es la reputación GANADA, no
la prueba de personalidad.** Crear identidades falsas o granjear avales en círculo no debe dar poder.

| Simulación | Resultado (ver `RESULTADOS.md`) |
|---|---|
| 200 Sybils (40% de la red) | capturan **0%** de la reputación y del consenso |
| Anillo de colusión de 30 | ~0% de poder de consenso |
| Lavado de reputación (1 real reparte a 29 títeres) | el damping anti-colusión lo recorta **~9×** |
| Blanqueo de seudónimo (whitewashing) | el seudónimo nuevo pierde **toda** la reputación |
| Slashing en cascada | el fraude del ahijado golpea a sus padrinos |

## Cómo se ejecuta

```bash
cd prototipos/reputacion
python3 run_all.py            # genera RESULTADOS.md
python3 run_all.py --stdout   # además lo imprime
python3 tests/test_motor.py   # autoauditoría (8 tests, sin pytest)
```

## Estructura

```
prototipos/reputacion/
├── harlequin_rep/          # el motor (paquete)
│   ├── model.py            # dimensiones (§1.2b) + agente con dos puertas (§1.4)
│   ├── graph.py            # grafo de avales + independencia anti-colusión (§1.6)
│   ├── reputacion.py       # EigenTrust anclado en evidencia + damping (§1.3, §1.6)
│   ├── consenso.py         # sorteo ponderado por reputación (§2.2)
│   └── vouch.py            # cupo, dividendo de mentor, slashing en cascada (§1.5c, §1.7)
├── escenarios.py           # poblaciones de red + ataques
├── run_all.py              # runner -> RESULTADOS.md
├── tests/test_motor.py     # tests de autoauditoría
└── RESULTADOS.md           # informe generado
```

## Cómo funciona (resumen técnico)

La reputación de cada dimensión es el vector estacionario de **EigenTrust** (Kamvar 2003) con
teletransporte a un **pre-trust anclado en evidencia real** (tratos liquidados, §1.3a) + semilla
génesis (§1.4):

```
t = (1 - α)·Cᵀ·t + α·p
```

- `C` = matriz de avales, fila-estocástica, **amortiguada por independencia** (§1.6): un aval entre
  miembros de un anillo cerrado (recíproco + vecinos compartidos) vale casi nada.
- **Detalle clave:** la amortiguación se normaliza por la suma SIN amortiguar, de modo que un nodo
  endogámico emite filas **sub-estocásticas**; la confianza que "no propaga" **se fuga al
  pre-trust**. Esto es lo que impide que un anillo recircule y amplifique reputación. (Si se
  normalizase por la suma amortiguada, un factor uniforme se cancelaría y el damping no haría nada
  — fue un bug real, corregido; ver el test `test_damping_reduce_lavado`.)
- `p` = pre-trust = evidencia normalizada. Sin evidencia real, ningún juego de avales fabrica poder.

El **consenso** (§2.2) sortea comités con probabilidad ∝ agregado **conservador** del vector
(mínimos/medianas, nunca suma: no se compra integridad con pericia, §1.2b).

## Limitaciones (honestas) y siguiente iteración

- El damping usa un clique denso como modelo de colusión. La colusión real es más sutil (anillos
  dispersos, avales asimétricos, ataques tipo PageRank). **Frente #1 de la SPEC (§1.6): seguir
  endureciendo.**
- El VRF se simula con un PRNG sembrado; en producción es una función aleatoria verificable real.
- Las cifras son **relativas** (reparto de poder), no parámetros de producción (esos son `[PARÁMETRO]`
  en la SPEC, a derivar con modelado).
- Falta modelar la dinámica temporal completa (decaimiento época a época, graduación de ahijados).

Estas se abordan en la próxima iteración del prototipo, antes de pasar al diseño de la cadena.
