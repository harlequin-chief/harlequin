<div align="center">

# Harlequin

**A security that cannot be bought.**

*Una seguridad que no se compra.*

</div>

---

## English

Harlequin is a project for making people freer and more independent — built in layers, without haste,
quality over speed. Its public charter (the Manifesto) lives at **harlequinproject.org**.

This repository holds the **open technical work**: the design of Harlequin's blockchain and, first of
all, its heart — the **reputation engine**.

### Why this is different

Every blockchain rests on one question: *what scarce good keeps an attacker from seizing the network?*

- **Bitcoin** makes the attack dear with **energy** (proof of work).
- **Ethereum** makes it dear with **money** (proof of stake).

In both, **power can be bought.** Harlequin proposes another foundation: **earned reputation over
time** — a good that is *not bought, not computed, and not hoarded*. We call the consensus built upon
it **Woven Trust Consensus**, and the unit of attack cost **reputation-time**.

### What's here

| Path | What it is |
|---|---|
| [`docs/woven-trust-consensus.md`](docs/woven-trust-consensus.md) | The consensus paper (English). |
| [`docs/consenso-confianza-tejida.md`](docs/consenso-confianza-tejida.md) | The consensus paper (Spanish). |
| [`reputation-engine/`](reputation-engine/) | Runnable prototype of the reputation core + attack simulations. |

### Run the reputation engine

No external dependencies — Python 3 standard library only.

```bash
cd reputation-engine
python3 run_all.py            # generates RESULTADOS.md
python3 tests/test_motor.py   # self-audit (8 tests)
```

It shows, in numbers, that false identities and collusion rings capture ~0% of consensus power, that
the anti-collusion damping is measurable, and that abandoning a pseudonym forfeits all earned standing.

### Honesty

This is research in progress, stated plainly. The reputation core runs and is measured; formal safety
proofs and resistance to *subtle* collusion remain open problems. We do not claim what we have not
shown.

### License

**GNU AGPL-3.0-or-later** — see [`LICENSE`](LICENSE). Chosen so the code stays free forever: any
derivative, even one offered only as a network service, must remain open.

---

## Español

Harlequin es un proyecto para hacer a las personas más libres e independientes — construido por capas,
sin prisa, calidad sobre velocidad. Su carta pública (el Manifiesto) vive en **proyectoharlequin.org**.

Este repositorio guarda el **trabajo técnico abierto**: el diseño de la blockchain de Harlequin y, ante
todo, su corazón — el **motor de reputación**.

### Por qué es diferente

Toda blockchain descansa en una pregunta: *¿qué bien escaso impide que un atacante se adueñe de la red?*

- **Bitcoin** encarece el ataque con **energía** (prueba de trabajo).
- **Ethereum** lo encarece con **dinero** (prueba de participación).

En ambas, **el poder se compra.** Harlequin propone otro cimiento: la **reputación ganada en el
tiempo** — un bien que *no se compra, no se computa y no se atesora*. Al consenso que se levanta sobre
él lo llamamos **Consenso de Confianza Tejida**, y a la unidad del coste de un ataque,
**reputación-tiempo**.

### Qué hay aquí

| Ruta | Qué es |
|---|---|
| [`docs/consenso-confianza-tejida.md`](docs/consenso-confianza-tejida.md) | El paper del consenso (español). |
| [`docs/woven-trust-consensus.md`](docs/woven-trust-consensus.md) | El paper del consenso (inglés). |
| [`reputation-engine/`](reputation-engine/) | Prototipo ejecutable del núcleo de reputación + simulaciones de ataque. |

### Ejecutar el motor de reputación

Sin dependencias externas — solo biblioteca estándar de Python 3.

```bash
cd reputation-engine
python3 run_all.py            # genera RESULTADOS.md
python3 tests/test_motor.py   # autoauditoría (8 tests)
```

Muestra, en cifras, que las identidades falsas y los anillos de colusión capturan ~0% del poder de
consenso, que la amortiguación anti-colusión es medible, y que abandonar un seudónimo cuesta toda la
reputación ganada.

### Honestidad

Esto es investigación en curso, dicha sin rodeos. El núcleo de reputación corre y se mide; las pruebas
formales de seguridad y la resistencia a la colusión *fina* quedan como problemas abiertos. No
reclamamos lo que no hemos probado.

### Licencia

**GNU AGPL-3.0-or-later** — ver [`LICENSE`](LICENSE). Elegida para que el código siga libre para
siempre: toda derivada, incluso si solo se ofrece como servicio en red, debe permanecer abierta.
