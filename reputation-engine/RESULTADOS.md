# RESULTADOS — Prototipo del motor de reputación de Harlequin
> Generado por `run_all.py` (solo stdlib). Reproducible: PRNG sembrado. Las cifras son
> **relativas** (reparto de poder), no unidades absolutas. Mapea SPEC §1 (reputación), §1.6
> (anti-colusión), §2.2 (consenso) y §5 (salida/seudónimo).

## Tesis que se pone a prueba
**El muro anti-Sybil real es la reputación GANADA, no la prueba de personalidad** (SPEC §1.5,
§2.4). Crear identidades falsas o granjear avales en círculo NO debe dar poder estructural,
porque la reputación se ancla en evidencia real y se amortigua la colusión (§1.6).

## 1. Reputación y poder de consenso por escenario
| Escenario | Nº agentes | Cuota de reputación ganada | Cuota de comité de consenso |
|---|---|---|---|
| **honesto** | 45 | genesis=8.47%, honesto=91.53% | genesis=9.75%, honesto=90.25% |
| **sybil** | 245 | genesis=8.47%, honesto=91.53% | genesis=9.75%, honesto=90.25% |
| **colusion** | 75 | genesis=8.05%, honesto=88.01%, colusor=3.94% | genesis=9.94%, honesto=90.06% |

**Lectura:**
- **Sybil:** 200 cuentas falsas (40% de la red) capturan **0.00%** de la reputación y **0.00%** del comité de consenso. Nacen con reputación 0; sin evidencia ni avales de reputados, no obtienen poder.
- **Colusión:** el anillo de 30 que se avala en círculo captura **3.94%** de la reputación y **0.00%** del comité, pese a los 3 honestos engañados que lo avalan.

## 2. ¿Cuánto aporta el anti-colusión (damping, §1.6)?
Ataque de **lavado de reputación**: un miembro del anillo (`c0`) SÍ tiene reputación legítima
alta (trabajo real) e intenta **repartirla** a 29 secuaces avalándose en círculo. El damping
debe impedir la propagación: `c0` conserva lo suyo, los secuaces se quedan en ~0.

| | Reputación de `c0` (legítima) | Repartida a los 29 secuaces | Máx. de un secuaz |
|---|---|---|---|
| **SIN damping** | 83.7 | 213.9 | 17.5 |
| **CON damping** | 133.2 | 24.6 | 12.7 |

El damping recorta la reputación lavada a los secuaces **8.7×**.
Sin el anti-colusión, un solo miembro con reputación real puede 'prestársela' a todo su
anillo de títeres; con él, la reputación se queda donde se ganó.

## 2b. Frente abierto: colusión SOFISTICADA (anillo disperso, §1.6)
El clique denso es fácil de detectar. Un anillo **disperso** (cada colusor avala a pocos, baja reciprocidad/solapamiento) imita patrones honestos y **se filtra** del damping local. Probamos una defensa nueva: **detección de comunidades** (la señal global que el anillo disperso sí deja).

| Defensa sobre el anillo disperso | Lavado a los secuaces |
|---|---|
| sin damping (referencia) | 209.5 |
| damping LOCAL (solo independencia pareja) | 148.0 |
| damping LOCAL + COMUNIDAD (nuevo) | 66.2 |

(Referencia: el clique denso con damping local lava solo 24.6.)

**Resultado:** la detección de comunidades reduce el lavado del anillo disperso de **148** (damping local) a **66** (~2.2× menos) — cierra buena parte del hueco que dejaba el damping local.
**Control de falsos positivos:** la reputación honesta total apenas cambia con la defensa de comunidades (-0.1% sobre la base) → no castiga a las comunidades honestas (tienen evidencia real, baja su sospecha). Honesto: sigue siendo opt-in y a validar más; la colusión adaptativa (fragmentar el anillo en varias comunidades) es el siguiente frente (§1.6, PAPER §10).

## 2c. Frente abierto: colusión ADAPTATIVA (fragmentar para evadir, §1.6)
El atacante sabe que castigamos las comunidades densas-sin-evidencia, así que **fragmenta** el anillo en sub-anillos pequeños y dispersos para caer por debajo del radar. Pero para lavar la reputación real de c0 a los títeres, ésta tiene que **fluir** entre fragmentos por unos pocos puentes. Esa es la tensión: evadir la etiqueta estrangula el flujo.

| fragmentos | comunidades vistas | lavado sin damping | lavado +comunidad | **poder consenso (min)** |
|---:|---:|---:|---:|---:|
| 1 | 1 | 209.5 | 66.2 | **0.000** |
| 2 | 2 | 214.8 | 61.3 | **0.000** |
| 3 | 3 | 239.3 | 62.1 | **0.000** |
| 5 | 5 | 200.9 | 53.2 | **0.000** |
| 6 | 6 | 209.4 | 56.0 | **0.000** |
| 10 | 6 | 264.6 | 42.4 | **0.000** |
| 15 | 3 | 202.9 | 49.2 | **0.000** |

**Resultado (doble):** (1) fragmentar **sube** el nº de comunidades vistas (evade la etiqueta) pero **no sube** el lavado bajo defensa —al fragmentar, los puentes se vuelven cuellos de botella que el damping LOCAL muerde más fuerte—; el peor caso para la defensa es el anillo disperso SIN fragmentar. (2) Lo que sí se filtra es **unidimensional** (solo `comercio`): bajo el agregado conservador (min, §1.2b) que rige el poder de consenso/aval, los títeres colapsan a **~0** a cualquier fragmentación. Para tener poder real el atacante necesitaría evidencia verificable en **todas** las dimensiones por cada títere = hacer el trabajo honesto. *El lavado difuso no compra poder estructural.*

## 3. Blanqueo de seudónimo (whitewashing, §5)
Mismo humano, dos máscaras: una consolidada y una nueva.

| Seudónimo | Reputación ganada |
|---|---|
| consolidado (`blanq_viejo`) | 59.23 |
| nuevo (`blanq_nuevo`) | 0.00 |

Abandonar el seudónimo para 'empezar limpio' cuesta **toda** la reputación ganada: vuelve a
la ciudadanía base. La reputación se ata a la máscara y no se transfiere (Art. VII/VIII).

## 4. Responsabilidad persistente + slashing en cascada (§1.5c, §1.7)
El ahijado defrauda y pierde 100. El golpe sube por la cadena de avales (½ por salto).

| Agente | Antes | Después | Δ |
|---|---|---|---|
| ahijado | 100.0 | 0.0 | -100.0 |
| intermedio | 120.0 | 70.0 | -50.0 |
| mentor | 200.0 | 175.0 | -25.0 |
| ajeno | 150.0 | 150.0 | +0.0 |

Quien avala responde de a quién metió, aunque pase el tiempo. El `ajeno` (no avaló) no se
ve afectado. Apadrinar a la ligera sale caro -> fuerza selectividad.

## 5. Economía de avales (§1.5c)
**Cupo de avales vivos = función sublineal de la reputación** (rendimientos decrecientes):

| Reputación | Cupo de avales |
|---|---|
| 1 | 3 |
| 10 | 10 |
| 100 | 19 |
| 1000 | 29 |
| 10000 | 39 |

**Dividendo de mentor:** apadrinar a alguien con reputación independiente real rinde 4.00; apadrinar a un títere (reputación independiente ~0) rinde 0.03. Las granjas padrino->títere no son rentables (§1.6).

## 6. Dinámica temporal: decaimiento y anti-atrincheramiento (§1.7, Art. VI)
El motor base es una foto fija; aquí se modela el TIEMPO en épocas envejeciendo el ancla de evidencia (ρ=0.7 de retención por época). La reputación no contribuida se evapora (§1.7) y el poder de ayer no blinda el de mañana (Art. VI).

| época | Honesto activo | Honesto retirado (para en t=3) | Pionero durmiente (obra única t=0) | Títeres anillo farm-y-sienta |
|---:|---:|---:|---:|---:|
| 0 | 151 | 240 | 816 | 19 |
| 1 | 215 | 327 | 566 | 28 |
| 2 | 283 | 415 | 436 | 22 |
| 3 | 364 | 372 | 359 | 19 |
| 4 | 438 | 333 | 292 | 15 |
| 5 | 501 | 300 | 234 | 12 |
| 6 | 553 | 274 | 188 | 9 |
| 7 | 594 | 253 | 152 | 7 |
| 8 | 626 | 236 | 124 | 5 |
| 9 | 650 | 224 | 104 | 4 |

**Resultado:** quien **sigue aportando** se sostiene y crece; quien se **retira** decae (~46% desde su pico) → anti-atrincheramiento (Art. VI). Un **pionero** de una obra única se desinfla (~87%) → defensa **anti-long-range** gratis: una historia vieja no se reactiva a poder. Y una **granja** que farmea y se sienta ve a sus títeres evaporarse: la colusión tiene que ser SOSTENIDA, no un sprint.

## 7. Graduación de ahijados: el apadrinamiento es andamiaje (§1.5c)
Un ahijado entra apadrinado (su reputación se apoya en el aval del mentor) y, al hacer obra real y recibir avales independientes, gradúa: el aval del mentor se libera y deja de ocupar su cupo. La responsabilidad persiste (slashing en cascada). Se mide la reputación de A CON vs SIN el aval del mentor (la independiente).

| época | rep total A | rep independiente A | % independiente | cupo libre M | graduado |
|---:|---:|---:|---:|---:|:--:|
| 0 | 162 | 0 | 0% | 22 | — |
| 1 | 216 | 76 | 35% | 22 | — |
| 2 | 276 | 158 | 57% | 21 | — |
| 3 | 315 | 219 | 70% | 22 | sí |
| 4 | 371 | 290 | 78% | 21 | sí |
| 5 | 425 | 357 | 84% | 21 | sí |
| 6 | 468 | 414 | 88% | 20 | sí |
| 7 | 512 | 464 | 91% | 20 | sí |

**Resultado:** A arranca dependiente del andamiaje (rep independiente ~0) y en la **época 3** se sostiene solo (≥60% independiente) → **gradúa**, liberando el cupo del mentor (21→22). El apadrinamiento bien hecho invierte en que el ahijado se **independice**, no en atarlo; el andamiaje se diseña para diluirse (coherente con la semilla génesis y el Art. VI).

## Conclusión
El prototipo confirma, en cifras, la apuesta central de la SPEC: **el poder estructural no se
compra con identidades ni con avales endogámicos**. Sybils y anillos de colusión quedan cerca
de 0% de poder de consenso; el damping anti-colusión es medible y necesario; el blanqueo no
compensa; y la responsabilidad persistente hace cara la colusión. La colusión sofisticada
—anillo disperso (§2b) y fragmentado/adaptativo (§2c)— tampoco compra poder: el lavado que se
filtra es unidimensional y el agregado conservador (§1.2b) lo anula. Frentes vivos: dinámica
temporal (decaimiento por época) y safety/liveness formales del consenso.
