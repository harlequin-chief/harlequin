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
├── adaptativo.py           # barrido de colusión adaptativa (fragmentar para evadir, §1.6)
├── temporal.py             # dinámica multi-época: decaimiento §1.7 / anti-atrincheramiento Art. VI
├── graduacion.py           # graduación de ahijados: el apadrinamiento se diluye (§1.5c)
├── aristas.py              # envejecimiento de avales por época: la confianza es perecedera (§1.7)
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

- **Colusión sofisticada — abordada (§2b/§2c del informe).** Además del clique denso, se modela el
  anillo **disperso** (defensa por detección de comunidades) y el **adaptativo** que fragmenta el
  anillo para evadir esa etiqueta (`adaptativo.py`). Hallazgo: fragmentar evade la etiqueta pero
  estrangula el flujo de reputación, y lo que se filtra es **unidimensional** → bajo el agregado
  conservador (§1.2b) el poder de consenso de los títeres colapsa a ~0 a cualquier fragmentación.
- **Colusión asimétrica — gap honesto documentado (§2d del informe).** Un **embudo PageRank** (muchos
  feeders avalan a un solo objetivo, sin reciprocidad) **evade el damping local**: el objetivo no
  avala a nadie, así que no deja firmas de reciprocidad ni solapamiento, y `independencia(feeder→c0)=1`.
  El pump pasa el damping de grafo, pero es unidimensional → el agregado conservador (§1.2b) lo deja en
  ~0 poder de consenso (el backstop que aguanta donde el grafo no llega). *Frente vivo:* endurecer la
  independencia con una señal de concentración de in-degree desde una sola comunidad (con análisis de
  falsos positivos sobre nodos honestos legítimamente populares).
- **Dinámica temporal — abordada (§6 del informe, `temporal.py`).** Simulación multi-época con
  envejecimiento del ancla de evidencia: quien deja de aportar decae (anti-atrincheramiento, Art.
  VI), un pionero de obra única no conserva poder (anti-long-range gratis), una granja que farmea y
  se sienta se desinfla.
- **Graduación de ahijados — abordada (§7 del informe, `graduacion.py`).** Un ahijado entra apoyado
  en el aval del mentor y, al ganar reputación independiente, gradúa: el aval se libera y deja de
  ocupar el cupo del mentor (la responsabilidad persiste). El andamiaje se diseña para diluirse.
- **Envejecimiento de avales — abordado (§8 del informe, `aristas.py`).** Además de la evidencia, los
  avales decaen con el tiempo salvo renovación. Experimento controlado (misma evidencia, distinta
  frescura de avales): prima de frescura ~200%; el aging añade decaimiento relativo sobre el control
  sin envejecer. Matiz honesto: el decaimiento uniforme se cancela en parte al normalizar la fila, así
  que el efecto real es RELATIVO (no renovar mientras otros sí) — lo que se quiere premiar.
- El VRF se simula con un PRNG sembrado; en producción es una función aleatoria verificable real.
- Las cifras son **relativas** (reparto de poder), no parámetros de producción (esos son `[PARÁMETRO]`
  en la SPEC, a derivar con modelado).

Frentes vivos: anti-colusión con coste económico explícito, y safety/liveness formales del consenso.
