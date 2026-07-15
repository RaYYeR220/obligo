import {
  ComputeBudgetProgram,
  Connection,
  Keypair,
  PublicKey,
  Transaction,
  TransactionInstruction,
  type AccountMeta,
  type Commitment,
} from '@solana/web3.js';
import {
  OBLIGO_PROGRAM_ID,
  OBLIGO_HOOK_ID,
  SYSTEM_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  TOKEN_2022_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} from './constants.ts';
import {
  authorityPda,
  batchPda,
  eamlPda,
  merchantPda,
  obligationPda,
  offerPda,
  permitPda,
  pointsAta,
  pointsMintPda,
  protocolPda,
  redemptionEscrow,
  vaultPda,
} from './pdas.ts';
import {
  decodeMerchant,
  decodeObligation,
  decodeOffer,
  decodePointBatch,
  decodeProtocol,
  enc,
  ixData,
  type AcceptanceOffer,
  type Merchant,
  type Obligation,
  type PointBatch,
  type Protocol,
} from './coder.ts';
import {
  clearCycleRemainingAccounts,
  findClearableCycle,
  type ClearableCycle,
  type ObligationEdge,
} from './cycles.ts';

const rw = (pubkey: PublicKey): AccountMeta => ({ pubkey, isSigner: false, isWritable: true });
const ro = (pubkey: PublicKey): AccountMeta => ({ pubkey, isSigner: false, isWritable: false });
const signerRw = (pubkey: PublicKey): AccountMeta => ({ pubkey, isSigner: true, isWritable: true });
const signerRo = (pubkey: PublicKey): AccountMeta => ({ pubkey, isSigner: true, isWritable: false });

export interface ObligoClientOpts {
  programId?: PublicKey;
  hookId?: PublicKey;
  /** The settlement asset. Filled in for you by `ObligoClient.load`. */
  usdcMint?: PublicKey;
}

export interface SendOpts {
  computeUnits?: number;
  priorityMicroLamports?: number;
  commitment?: Commitment;
  /** Defaults to `signers[0]`. Must be one of `signers`. */
  feePayer?: PublicKey;
}

/**
 * A typed client for the Obligo core program.
 *
 * Every instruction method returns a plain `TransactionInstruction`, so an integrator can compose it
 * with their own — a merchant's till batching a checkout, a relayer paying the fee, a crank bundling
 * a settle after a redeem. `sendAndConfirm` is there when you just want it to land.
 *
 * There is no IDL (the toolchain's anchor-cli is version-mismatched); the encodings here are the
 * hand-rolled ones the devnet transcript proved, packaged as a reusable module.
 */
export class ObligoClient {
  readonly connection: Connection;
  readonly programId: PublicKey;
  hookId: PublicKey;
  usdcMint?: PublicKey;

  constructor(connection: Connection, opts: ObligoClientOpts = {}) {
    this.connection = connection;
    this.programId = opts.programId ?? OBLIGO_PROGRAM_ID;
    this.hookId = opts.hookId ?? OBLIGO_HOOK_ID;
    this.usdcMint = opts.usdcMint;
  }

  /** Construct a client and read the on-chain protocol to learn its settlement mint and hook. */
  static async load(connection: Connection, opts: ObligoClientOpts = {}): Promise<ObligoClient> {
    const client = new ObligoClient(connection, opts);
    const protocol = await client.getProtocol();
    if (protocol) {
      client.usdcMint = opts.usdcMint ?? protocol.usdcMint;
      client.hookId = opts.hookId ?? protocol.hookProgram;
    }
    return client;
  }

  private requireUsdc(): PublicKey {
    if (!this.usdcMint) {
      throw new Error(
        'usdcMint is unknown — build the client with `{ usdcMint }` or use `ObligoClient.load()`.',
      );
    }
    return this.usdcMint;
  }

  // ---- PDA passthroughs (bound to this client's program + hook) -----------------------------

  protocol(): PublicKey {
    return protocolPda(this.programId);
  }
  authority(): PublicKey {
    return authorityPda(this.programId);
  }
  merchant(authority: PublicKey): PublicKey {
    return merchantPda(authority, this.programId);
  }
  vault(merchant: PublicKey): PublicKey {
    return vaultPda(merchant, this.programId);
  }
  pointsMint(merchant: PublicKey): PublicKey {
    return pointsMintPda(merchant, this.programId);
  }
  offer(acceptor: PublicKey, issuer: PublicKey): PublicKey {
    return offerPda(acceptor, issuer, this.programId);
  }
  obligation(debtor: PublicKey, creditor: PublicKey): PublicKey {
    return obligationPda(debtor, creditor, this.programId);
  }

  // ---- instruction builders -----------------------------------------------------------------

  initProtocol(p: { authority: PublicKey; usdcMint: PublicKey }): TransactionInstruction {
    return new TransactionInstruction({
      programId: this.programId,
      keys: [
        signerRw(p.authority),
        rw(this.protocol()),
        ro(p.usdcMint),
        ro(this.hookId),
        ro(this.authority()),
        ro(SYSTEM_PROGRAM_ID),
      ],
      data: ixData('init_protocol'),
    });
  }

  registerMerchant(p: {
    authority: PublicKey;
    name: string;
    usdcPerPoint: bigint | number;
    reserveBps: number;
    pointTtl: bigint | number;
  }): TransactionInstruction {
    const merchant = this.merchant(p.authority);
    return new TransactionInstruction({
      programId: this.programId,
      keys: [
        signerRw(p.authority),
        rw(this.protocol()),
        rw(merchant),
        ro(this.requireUsdc()),
        rw(this.vault(merchant)),
        ro(TOKEN_PROGRAM_ID),
        ro(SYSTEM_PROGRAM_ID),
      ],
      data: ixData(
        'register_merchant',
        enc.str(p.name),
        enc.u64(p.usdcPerPoint),
        enc.u16(p.reserveBps),
        enc.i64(p.pointTtl),
      ),
    });
  }

  setTerms(p: {
    authority: PublicKey;
    usdcPerPoint: bigint | number;
    reserveBps: number;
    pointTtl: bigint | number;
  }): TransactionInstruction {
    return new TransactionInstruction({
      programId: this.programId,
      keys: [signerRo(p.authority), rw(this.merchant(p.authority))],
      data: ixData(
        'set_terms',
        enc.u64(p.usdcPerPoint),
        enc.u16(p.reserveBps),
        enc.i64(p.pointTtl),
      ),
    });
  }

  /** Permissionless: anyone may back any merchant. `merchant` is the merchant PDA. */
  depositCollateral(p: {
    depositor: PublicKey;
    merchant: PublicKey;
    fromUsdc: PublicKey;
    amount: bigint | number;
  }): TransactionInstruction {
    return new TransactionInstruction({
      programId: this.programId,
      keys: [
        signerRo(p.depositor),
        rw(p.merchant),
        rw(this.vault(p.merchant)),
        ro(this.requireUsdc()),
        rw(p.fromUsdc),
        ro(TOKEN_PROGRAM_ID),
      ],
      data: ixData('deposit_collateral', enc.u64(p.amount)),
    });
  }

  withdrawCollateral(p: {
    authority: PublicKey;
    destination: PublicKey;
    amount: bigint | number;
  }): TransactionInstruction {
    const merchant = this.merchant(p.authority);
    return new TransactionInstruction({
      programId: this.programId,
      keys: [
        signerRo(p.authority),
        rw(merchant),
        rw(this.vault(merchant)),
        ro(this.requireUsdc()),
        rw(p.destination),
        ro(TOKEN_PROGRAM_ID),
      ],
      data: ixData('withdraw_collateral', enc.u64(p.amount)),
    });
  }

  createPointsMint(p: {
    authority: PublicKey;
    name: string;
    symbol: string;
    uri: string;
  }): TransactionInstruction {
    const merchant = this.merchant(p.authority);
    const mint = this.pointsMint(merchant);
    return new TransactionInstruction({
      programId: this.programId,
      keys: [
        signerRw(p.authority),
        ro(this.protocol()),
        rw(merchant),
        rw(mint),
        rw(eamlPda(mint, this.hookId)),
        ro(this.hookId),
        ro(TOKEN_2022_PROGRAM_ID),
        ro(SYSTEM_PROGRAM_ID),
      ],
      data: ixData('create_points_mint', enc.str(p.name), enc.str(p.symbol), enc.str(p.uri)),
    });
  }

  issuePoints(p: {
    authority: PublicKey;
    customer: PublicKey;
    amount: bigint | number;
  }): TransactionInstruction {
    const merchant = this.merchant(p.authority);
    const mint = this.pointsMint(merchant);
    return new TransactionInstruction({
      programId: this.programId,
      keys: [
        signerRw(p.authority),
        rw(merchant),
        rw(mint),
        ro(p.customer),
        rw(pointsAta(mint, p.customer)),
        rw(batchPda(merchant, p.customer, this.programId)),
        ro(TOKEN_2022_PROGRAM_ID),
        ro(ASSOCIATED_TOKEN_PROGRAM_ID),
        ro(SYSTEM_PROGRAM_ID),
      ],
      data: ixData('issue_points', enc.u64(p.amount)),
    });
  }

  /** `issuerMerchant` is the merchant PDA whose points the offer bids for. */
  postOffer(p: {
    acceptorAuthority: PublicKey;
    issuerMerchant: PublicKey;
    rateBps: number;
    capacity: bigint | number;
    expiresAt: bigint | number;
  }): TransactionInstruction {
    const acceptor = this.merchant(p.acceptorAuthority);
    return new TransactionInstruction({
      programId: this.programId,
      keys: [
        signerRw(p.acceptorAuthority),
        ro(acceptor),
        ro(p.issuerMerchant),
        rw(this.offer(acceptor, p.issuerMerchant)),
        ro(SYSTEM_PROGRAM_ID),
      ],
      data: ixData('post_offer', enc.u16(p.rateBps), enc.u64(p.capacity), enc.i64(p.expiresAt)),
    });
  }

  cancelOffer(p: {
    acceptorAuthority: PublicKey;
    issuerMerchant: PublicKey;
  }): TransactionInstruction {
    const acceptor = this.merchant(p.acceptorAuthority);
    return new TransactionInstruction({
      programId: this.programId,
      keys: [
        signerRw(p.acceptorAuthority),
        ro(acceptor),
        rw(this.offer(acceptor, p.issuerMerchant)),
      ],
      data: ixData('cancel_offer'),
    });
  }

  /**
   * A customer spends `issuerMerchant`'s points at `acceptorMerchant`. The acceptor does not sign —
   * its posted offer is its consent — so only `payer` and `customer` sign. `payer` may be a till or
   * a relayer; the customer needs no lamports.
   */
  redeem(p: {
    payer: PublicKey;
    customer: PublicKey;
    issuerMerchant: PublicKey;
    acceptorMerchant: PublicKey;
    points: bigint | number;
  }): TransactionInstruction {
    const mint = this.pointsMint(p.issuerMerchant);
    const customerPoints = pointsAta(mint, p.customer);
    return new TransactionInstruction({
      programId: this.programId,
      keys: [
        signerRw(p.payer),
        signerRo(p.customer),
        ro(this.protocol()),
        rw(p.issuerMerchant),
        rw(p.acceptorMerchant),
        rw(this.offer(p.acceptorMerchant, p.issuerMerchant)),
        rw(this.obligation(p.issuerMerchant, p.acceptorMerchant)),
        rw(mint),
        rw(customerPoints),
        rw(redemptionEscrow(p.issuerMerchant, mint)),
        rw(batchPda(p.issuerMerchant, p.customer, this.programId)),
        ro(this.authority()),
        rw(permitPda(customerPoints, this.hookId)),
        ro(eamlPda(mint, this.hookId)),
        ro(this.hookId),
        ro(TOKEN_2022_PROGRAM_ID),
        ro(ASSOCIATED_TOKEN_PROGRAM_ID),
        ro(SYSTEM_PROGRAM_ID),
      ],
      data: ixData('redeem', enc.u64(p.points)),
    });
  }

  /** Net two merchants' mutual debt and move only the difference. `merchantA`/`B` are PDAs. */
  settle(p: {
    cranker: PublicKey;
    merchantA: PublicKey;
    merchantB: PublicKey;
  }): TransactionInstruction {
    return new TransactionInstruction({
      programId: this.programId,
      keys: [
        signerRw(p.cranker),
        ro(this.protocol()),
        rw(p.merchantA),
        rw(p.merchantB),
        rw(this.vault(p.merchantA)),
        rw(this.vault(p.merchantB)),
        rw(this.obligation(p.merchantA, p.merchantB)),
        rw(this.obligation(p.merchantB, p.merchantA)),
        ro(this.requireUsdc()),
        ro(TOKEN_PROGRAM_ID),
        ro(SYSTEM_PROGRAM_ID),
      ],
      data: ixData('settle'),
    });
  }

  /** Cancel a ring of debt. Pass the `ClearableCycle` from `findClearableCycle`. */
  clearCycle(p: { cranker: PublicKey; cycle: ClearableCycle }): TransactionInstruction {
    return new TransactionInstruction({
      programId: this.programId,
      keys: [signerRw(p.cranker), ...clearCycleRemainingAccounts(p.cycle)],
      data: ixData('clear_cycle', enc.u8(p.cycle.ring.length)),
    });
  }

  liquidate(p: {
    cranker: PublicKey;
    debtor: PublicKey;
    creditor: PublicKey;
  }): TransactionInstruction {
    return new TransactionInstruction({
      programId: this.programId,
      keys: [
        signerRw(p.cranker),
        ro(this.protocol()),
        rw(p.debtor),
        rw(p.creditor),
        rw(this.vault(p.debtor)),
        rw(this.vault(p.creditor)),
        rw(this.obligation(p.debtor, p.creditor)),
        ro(this.requireUsdc()),
        ro(TOKEN_PROGRAM_ID),
      ],
      data: ixData('liquidate'),
    });
  }

  reinstate(p: { cranker: PublicKey; merchant: PublicKey }): TransactionInstruction {
    return new TransactionInstruction({
      programId: this.programId,
      keys: [signerRw(p.cranker), rw(p.merchant)],
      data: ixData('reinstate'),
    });
  }

  /** Burn a customer's lapsed points. Nobody signs for the customer — the merchant PDA is the
   * mint's permanent delegate, and the movement still goes through the hook under a permit. */
  expirePoints(p: {
    cranker: PublicKey;
    merchant: PublicKey;
    customer: PublicKey;
  }): TransactionInstruction {
    const mint = this.pointsMint(p.merchant);
    const customerPoints = pointsAta(mint, p.customer);
    return new TransactionInstruction({
      programId: this.programId,
      keys: [
        signerRw(p.cranker),
        ro(this.protocol()),
        rw(p.merchant),
        rw(mint),
        ro(p.customer),
        rw(customerPoints),
        rw(redemptionEscrow(p.merchant, mint)),
        rw(batchPda(p.merchant, p.customer, this.programId)),
        ro(this.authority()),
        rw(permitPda(customerPoints, this.hookId)),
        ro(eamlPda(mint, this.hookId)),
        ro(this.hookId),
        ro(TOKEN_2022_PROGRAM_ID),
        ro(ASSOCIATED_TOKEN_PROGRAM_ID),
        ro(SYSTEM_PROGRAM_ID),
      ],
      data: ixData('expire_points'),
    });
  }

  // ---- account reads ------------------------------------------------------------------------

  async getProtocol(): Promise<Protocol | null> {
    const ai = await this.connection.getAccountInfo(this.protocol());
    return ai ? decodeProtocol(ai.data) : null;
  }

  async getMerchant(merchant: PublicKey): Promise<Merchant | null> {
    const ai = await this.connection.getAccountInfo(merchant);
    return ai ? decodeMerchant(ai.data) : null;
  }

  async getMerchantByAuthority(authority: PublicKey): Promise<Merchant | null> {
    return this.getMerchant(this.merchant(authority));
  }

  async getOffer(acceptor: PublicKey, issuer: PublicKey): Promise<AcceptanceOffer | null> {
    const ai = await this.connection.getAccountInfo(this.offer(acceptor, issuer));
    return ai ? decodeOffer(ai.data) : null;
  }

  async getObligation(debtor: PublicKey, creditor: PublicKey): Promise<Obligation | null> {
    const ai = await this.connection.getAccountInfo(this.obligation(debtor, creditor));
    return ai ? decodeObligation(ai.data) : null;
  }

  /** What `debtor` owes `creditor`, with "no edge at all" reading as the zero it means. */
  async getObligationAmount(debtor: PublicKey, creditor: PublicKey): Promise<bigint> {
    const ob = await this.getObligation(debtor, creditor);
    return ob ? ob.amount : 0n;
  }

  async getPointBatch(merchant: PublicKey, customer: PublicKey): Promise<PointBatch | null> {
    const ai = await this.connection.getAccountInfo(batchPda(merchant, customer, this.programId));
    return ai ? decodePointBatch(ai.data) : null;
  }

  /**
   * Read every live directed edge among a set of merchants — the obligation graph, as far as those
   * merchants participate in it. Feed it straight into `findClearableCycle`.
   */
  async loadObligationEdges(merchants: PublicKey[]): Promise<ObligationEdge[]> {
    const pairs: { debtor: PublicKey; creditor: PublicKey; pda: PublicKey }[] = [];
    for (const debtor of merchants) {
      for (const creditor of merchants) {
        if (debtor.equals(creditor)) continue;
        pairs.push({ debtor, creditor, pda: this.obligation(debtor, creditor) });
      }
    }
    const edges: ObligationEdge[] = [];
    // getMultipleAccountsInfo caps at 100 keys per call.
    for (let i = 0; i < pairs.length; i += 100) {
      const slice = pairs.slice(i, i + 100);
      const infos = await this.connection.getMultipleAccountsInfo(slice.map((p) => p.pda));
      infos.forEach((info, j) => {
        if (!info) return;
        const ob = decodeObligation(info.data);
        if (ob.amount > 0n) {
          edges.push({ debtor: slice[j].debtor, creditor: slice[j].creditor, amount: ob.amount });
        }
      });
    }
    return edges;
  }

  /** Load the graph among `merchants` and return the first clearable cycle in it, if any. */
  async findClearableCycle(
    merchants: PublicKey[],
    opts: { minLen?: number; maxLen?: number } = {},
  ): Promise<ClearableCycle | null> {
    const edges = await this.loadObligationEdges(merchants);
    return findClearableCycle(edges, { ...opts, programId: this.programId });
  }

  // ---- sending ------------------------------------------------------------------------------

  /**
   * Build, sign, send and confirm a transaction, with backoff around the public devnet RPC (which
   * rate-limits hard). The fee payer defaults to `signers[0]` and must be one of `signers`.
   */
  async sendAndConfirm(
    ixs: TransactionInstruction[],
    signers: Keypair[],
    opts: SendOpts = {},
  ): Promise<string> {
    const commitment: Commitment = opts.commitment ?? 'confirmed';
    const tx = new Transaction();
    if (opts.computeUnits) {
      tx.add(ComputeBudgetProgram.setComputeUnitLimit({ units: opts.computeUnits }));
    }
    if (opts.priorityMicroLamports) {
      tx.add(ComputeBudgetProgram.setComputeUnitPrice({ microLamports: opts.priorityMicroLamports }));
    }
    for (const ix of ixs) tx.add(ix);
    tx.feePayer = opts.feePayer ?? signers[0].publicKey;

    const { blockhash, lastValidBlockHeight } = await this.withRetry(() =>
      this.connection.getLatestBlockhash(commitment),
    );
    tx.recentBlockhash = blockhash;
    tx.sign(...signers);

    const raw = tx.serialize();
    const sig = await this.withRetry(() =>
      this.connection.sendRawTransaction(raw, {
        preflightCommitment: commitment,
        maxRetries: 5,
      }),
    );
    await this.confirmSignature(sig, lastValidBlockHeight, commitment);
    return sig;
  }

  /**
   * Build + send a transaction whose fee payer is signed by an external signer — a browser wallet's
   * `signTransaction`, a hardware device, a relayer — reusing the same devnet backoff as
   * `sendAndConfirm`. Any `extraSigners` (ancillary keypairs, e.g. a freshly-generated mint) are
   * partial-signed first; then `sign` is invoked to add the fee payer's signature and returns the
   * fully-signed transaction. The fee payer needs no `Keypair` on this side.
   */
  async sendSigned(
    ixs: TransactionInstruction[],
    feePayer: PublicKey,
    sign: (tx: Transaction) => Promise<Transaction>,
    opts: SendOpts & { extraSigners?: Keypair[] } = {},
  ): Promise<string> {
    const commitment: Commitment = opts.commitment ?? 'confirmed';
    const tx = new Transaction();
    if (opts.computeUnits) {
      tx.add(ComputeBudgetProgram.setComputeUnitLimit({ units: opts.computeUnits }));
    }
    if (opts.priorityMicroLamports) {
      tx.add(ComputeBudgetProgram.setComputeUnitPrice({ microLamports: opts.priorityMicroLamports }));
    }
    for (const ix of ixs) tx.add(ix);
    tx.feePayer = feePayer;

    const { blockhash, lastValidBlockHeight } = await this.withRetry(() =>
      this.connection.getLatestBlockhash(commitment),
    );
    tx.recentBlockhash = blockhash;
    if (opts.extraSigners && opts.extraSigners.length > 0) tx.partialSign(...opts.extraSigners);

    const signed = await sign(tx);

    const raw = signed.serialize();
    const sig = await this.withRetry(() =>
      this.connection.sendRawTransaction(raw, {
        preflightCommitment: commitment,
        maxRetries: 5,
      }),
    );
    await this.confirmSignature(sig, lastValidBlockHeight, commitment);
    return sig;
  }

  private async confirmSignature(
    signature: string,
    lastValidBlockHeight: number,
    commitment: Commitment,
  ): Promise<void> {
    for (;;) {
      const statuses = await this.withRetry(() =>
        this.connection.getSignatureStatuses([signature]),
      );
      const status = statuses.value[0];
      if (status) {
        if (status.err) {
          throw new Error(`transaction failed on-chain: ${JSON.stringify(status.err)}`);
        }
        if (status.confirmationStatus === commitment || status.confirmationStatus === 'finalized') {
          return;
        }
      }
      const height = await this.withRetry(() => this.connection.getBlockHeight(commitment));
      if (height > lastValidBlockHeight) {
        throw new Error('blockhash expired before the transaction confirmed');
      }
      await sleep(600);
    }
  }

  private async withRetry<T>(fn: () => Promise<T>): Promise<T> {
    let delay = 500;
    let lastErr: unknown;
    for (let i = 0; i < 10; i++) {
      try {
        return await fn();
      } catch (e) {
        lastErr = e;
        const msg = String((e as { message?: string })?.message ?? e).toLowerCase();
        const transient =
          msg.includes('429') ||
          msg.includes('too many') ||
          msg.includes('503') ||
          msg.includes('fetch failed') ||
          msg.includes('timeout');
        if (!transient) throw e;
        await sleep(delay);
        delay = Math.min(delay * 2, 8000);
      }
    }
    throw lastErr;
  }
}

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));
