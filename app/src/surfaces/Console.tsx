import { useCallback, useEffect, useState } from 'react';
import { PublicKey } from '@solana/web3.js';
import {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} from '@solana/spl-token';
import type { Merchant } from '@obligo/sdk';
import { useNet } from '../hooks/useNetworkData.tsx';
import { useWallet, fetchUsdc } from '../lib/wallet.tsx';
import { bps, healthColor, healthLabel, pct, shortAddr, usd } from '../lib/format.ts';
import { displayName } from '../lib/names.ts';
import { humaniseError, send, txUrl } from '../lib/tx.ts';
import { healthBps, isSolvent, requiredCollateral } from '@obligo/sdk';

type Toast = { kind: 'ok' | 'err'; text: string; sig?: string } | null;

export default function Console() {
  const { net, client, reload } = useNet();
  const wallet = useWallet();
  const [merchant, setMerchant] = useState<Merchant | null>(null);
  const [usdcBal, setUsdcBal] = useState<bigint>(0n);
  const [busy, setBusy] = useState<string | null>(null);
  const [toast, setToast] = useState<Toast>(null);

  const loadSelf = useCallback(async () => {
    if (!wallet.keypair) {
      setMerchant(null);
      return;
    }
    const m = await client.getMerchantByAuthority(wallet.keypair.publicKey);
    setMerchant(m);
    if (net?.protocol) {
      setUsdcBal(await fetchUsdc(net.protocol.usdcMint, wallet.keypair.publicKey));
    }
  }, [wallet.keypair, client, net?.protocol]);

  useEffect(() => {
    void loadSelf();
  }, [loadSelf, net?.loadedAt]);

  const run = async (label: string, fn: () => Promise<string>, note?: string) => {
    setBusy(label);
    setToast(null);
    try {
      const sig = await fn();
      setToast({ kind: 'ok', text: note ?? `${label} confirmed`, sig });
      await loadSelf();
      await reload();
    } catch (e) {
      setToast({ kind: 'err', text: humaniseError(e) });
    } finally {
      setBusy(null);
    }
  };

  if (!wallet.keypair) {
    return (
      <div className="scroll-surface" style={{ padding: 40 }}>
        <Empty />
      </div>
    );
  }

  const kp = wallet.keypair;
  const hasMint = !!merchant && !new PublicKey(merchant.pointsMint).equals(PublicKey.default);
  const mv = merchant
    ? { health: healthBps(merchant), solvent: isSolvent(merchant), required: requiredCollateral(merchant) }
    : null;

  return (
    <div className="scroll-surface" style={{ padding: '22px 26px', maxWidth: 1120, margin: '0 auto' }}>
      <div className="row spread center" style={{ marginBottom: 18 }}>
        <div>
          <div className="eyebrow">Merchant console</div>
          <h2 style={{ fontSize: 22, marginTop: 4 }}>{merchant ? displayName(merchant).label : 'Unregistered'} <span className="mono dim" style={{ fontSize: 12, fontWeight: 400 }}>· {shortAddr(kp.publicKey.toBase58(), 5)}</span></h2>
        </div>
        <div className="row gap-16">
          <MiniStat k="wallet sol" v={wallet.sol === null ? '…' : wallet.sol.toFixed(3) + ' ◎'} />
          <MiniStat k="test-usdc" v={usd(usdcBal)} accent={usdcBal > 0n ? 'var(--green)' : 'var(--ink-3)'} />
        </div>
      </div>

      {toast && (
        <div className={`banner ${toast.kind}`} style={{ marginBottom: 16 }}>
          <span>{toast.text}</span>
          {toast.sig && <a href={txUrl(toast.sig)} target="_blank" rel="noreferrer" style={{ marginLeft: 'auto' }}>{shortAddr(toast.sig, 6)} ↗</a>}
        </div>
      )}

      {/* books */}
      {merchant && mv && (
        <div className="panel" style={{ padding: 16, marginBottom: 18 }}>
          <div className="row spread center" style={{ marginBottom: 12 }}>
            <div className="row gap-8 center">
              <span className="dot" style={{ background: healthColor(mv.health, mv.solvent), width: 9, height: 9 }} />
              <span className="eyebrow">live books</span>
              <span className="hchip" style={{ color: healthColor(mv.health, mv.solvent) }}>{healthLabel(mv.health, mv.solvent)}</span>
            </div>
            <span className="mono dim" style={{ fontSize: 11 }}>health {pct(mv.health)}</span>
          </div>
          <div className="formgrid" style={{ gridTemplateColumns: 'repeat(4,1fr)', gap: 14 }}>
            <Book k="collateral" v={usd(merchant.collateral)} />
            <Book k="required" v={usd(mv.required)} />
            <Book k="owes (out)" v={usd(merchant.obligationsOut)} accent={merchant.obligationsOut > 0n ? 'var(--red-hi)' : undefined} />
            <Book k="owed (in)" v={usd(merchant.obligationsIn)} accent={merchant.obligationsIn > 0n ? 'var(--green-hi)' : undefined} />
            <Book k="points out" v={merchant.pointsOutstanding.toString()} />
            <Book k="terms" v={`${usd(merchant.usdcPerPoint)}/pt`} />
            <Book k="reserve" v={bps(merchant.reserveBps)} />
            <Book k="mint" v={hasMint ? 'live' : 'none'} accent={hasMint ? 'var(--green)' : 'var(--amber)'} />
          </div>
        </div>
      )}

      <div className="formgrid" style={{ gap: 18 }}>
        {!merchant ? (
          <RegisterCard busy={busy} run={run} client={client} kp={kp} />
        ) : (
          <>
            <DepositCard busy={busy} run={run} client={client} kp={kp} merchantPda={client.merchant(kp.publicKey)} usdcBal={usdcBal} usdcMint={net?.protocol?.usdcMint ?? null} />
            {!hasMint ? (
              <CreateMintCard busy={busy} run={run} client={client} kp={kp} name={displayName(merchant).label} />
            ) : (
              <IssueCard busy={busy} run={run} client={client} kp={kp} />
            )}
            <SetTermsCard busy={busy} run={run} client={client} kp={kp} merchant={merchant} />
            <OfferCard busy={busy} run={run} client={client} kp={kp} />
          </>
        )}
      </div>
    </div>
  );
}

/* ---------- action cards ---------- */

function Card({ title, sub, children }: { title: string; sub: string; children: React.ReactNode }) {
  return (
    <div className="panel" style={{ padding: 16 }}>
      <div className="eyebrow" style={{ color: 'var(--amber)' }}>{title}</div>
      <div className="mono dim" style={{ fontSize: 10.5, margin: '6px 0 14px', lineHeight: 1.5 }}>{sub}</div>
      {children}
    </div>
  );
}

function RegisterCard({ busy, run, client, kp }: any) {
  const [name, setName] = useState('CAFE');
  const [perPoint, setPerPoint] = useState('0.01');
  const [reserve, setReserve] = useState('30');
  const [ttl, setTtl] = useState('30');
  return (
    <Card title="register merchant" sub="Permissionless. No whitelist — the reserve invariant and public health restrain you, not an admin.">
      <div className="formgrid">
        <Field label="name (≤32)"><input value={name} maxLength={32} onChange={(e) => setName(e.target.value)} /></Field>
        <Field label="usdc / point ($)"><input value={perPoint} onChange={(e) => setPerPoint(e.target.value)} /></Field>
        <Field label="reserve (%)"><input value={reserve} onChange={(e) => setReserve(e.target.value)} /></Field>
        <Field label="point ttl (days)"><input value={ttl} onChange={(e) => setTtl(e.target.value)} /></Field>
      </div>
      <button className="btn btn-amber" style={{ width: '100%', marginTop: 6 }} disabled={!!busy}
        onClick={() => run('register', () => send(client, [client.registerMerchant({
          authority: kp.publicKey, name,
          usdcPerPoint: Math.round(parseFloat(perPoint) * 1e6),
          reserveBps: Math.round(parseFloat(reserve) * 100),
          pointTtl: Math.round(parseFloat(ttl) * 86400),
        })], [kp]))}>
        {busy === 'register' ? 'confirming…' : 'register on devnet'}
      </button>
    </Card>
  );
}

function DepositCard({ busy, run, client, kp, merchantPda, usdcBal, usdcMint }: any) {
  const [amt, setAmt] = useState('50');
  const disabled = !!busy || usdcBal <= 0n;
  return (
    <Card title="deposit collateral" sub="Backs your points. Partial collateral is the whole idea — escrow a fraction of face and let the market price your credit.">
      {usdcBal <= 0n && (
        <div className="banner warn" style={{ marginBottom: 12 }}>
          This key holds no test-USDC. The devnet settlement mint is dev-controlled, so deposits are gated to keys that hold it. Registration, mint creation and offers still work with SOL alone.
        </div>
      )}
      <Field label="amount ($)"><input value={amt} onChange={(e) => setAmt(e.target.value)} /></Field>
      <button className="btn btn-amber" style={{ width: '100%' }} disabled={disabled}
        onClick={() => run('deposit', () => {
          const fromUsdc = getAssociatedTokenAddressSync(usdcMint, kp.publicKey, false, TOKEN_PROGRAM_ID, ASSOCIATED_TOKEN_PROGRAM_ID);
          return send(client, [client.depositCollateral({ depositor: kp.publicKey, merchant: merchantPda, fromUsdc, amount: Math.round(parseFloat(amt) * 1e6) })], [kp]);
        })}>
        {busy === 'deposit' ? 'confirming…' : 'deposit'}
      </button>
    </Card>
  );
}

function CreateMintCard({ busy, run, client, kp, name }: any) {
  const [sym, setSym] = useState('PTS');
  return (
    <Card title="create points mint" sub="A Token-2022 mint bound to the Obligo hook. Hook authority is burned at creation — the rules can never be repointed.">
      <div className="formgrid">
        <Field label="metadata name"><input defaultValue={`${name} Points`} id="mintname" /></Field>
        <Field label="symbol (≤10)"><input value={sym} maxLength={10} onChange={(e) => setSym(e.target.value)} /></Field>
      </div>
      <button className="btn btn-amber" style={{ width: '100%', marginTop: 6 }} disabled={!!busy}
        onClick={() => run('mint', () => send(client, [client.createPointsMint({
          authority: kp.publicKey,
          name: (document.getElementById('mintname') as HTMLInputElement).value,
          symbol: sym,
          uri: `https://obligo.xyz/${sym}.json`,
        })], [kp], 600_000))}>
        {busy === 'mint' ? 'confirming…' : 'create Token-2022 mint'}
      </button>
    </Card>
  );
}

function IssueCard({ busy, run, client, kp }: any) {
  const [cust, setCust] = useState('');
  const [amt, setAmt] = useState('500');
  let valid = false;
  try { if (cust) { new PublicKey(cust); valid = true; } } catch { valid = false; }
  return (
    <Card title="issue points" sub="Mint points to a customer under the reserve invariant. Needs collateral — issuance is refused the moment you can't back it.">
      <Field label="customer address">
        <input value={cust} onChange={(e) => setCust(e.target.value)} placeholder="paste a wallet address" />
      </Field>
      <div className="row gap-8" style={{ alignItems: 'flex-end' }}>
        <Field label="points"><input value={amt} onChange={(e) => setAmt(e.target.value)} /></Field>
        <button className="btn btn-ghost" style={{ marginBottom: 14 }} onClick={() => setCust(kp.publicKey.toBase58())}>me</button>
      </div>
      <button className="btn btn-amber" style={{ width: '100%' }} disabled={!!busy || !valid}
        onClick={() => run('issue', () => send(client, [client.issuePoints({ authority: kp.publicKey, customer: new PublicKey(cust), amount: Math.round(parseFloat(amt)) })], [kp]))}>
        {busy === 'issue' ? 'confirming…' : 'issue'}
      </button>
    </Card>
  );
}

function SetTermsCard({ busy, run, client, kp, merchant }: any) {
  const [perPoint, setPerPoint] = useState((Number(merchant.usdcPerPoint) / 1e6).toString());
  const [reserve, setReserve] = useState((merchant.reserveBps / 100).toString());
  const [ttl, setTtl] = useState((Number(merchant.pointTtl) / 86400).toString());
  return (
    <Card title="set terms" sub="Re-price your own risk. You cannot re-price points already in the wild — only new issuance.">
      <div className="formgrid">
        <Field label="usdc / point ($)"><input value={perPoint} onChange={(e) => setPerPoint(e.target.value)} /></Field>
        <Field label="reserve (%)"><input value={reserve} onChange={(e) => setReserve(e.target.value)} /></Field>
      </div>
      <Field label="point ttl (days)"><input value={ttl} onChange={(e) => setTtl(e.target.value)} /></Field>
      <button className="btn" style={{ width: '100%' }} disabled={!!busy}
        onClick={() => run('terms', () => send(client, [client.setTerms({
          authority: kp.publicKey,
          usdcPerPoint: Math.round(parseFloat(perPoint) * 1e6),
          reserveBps: Math.round(parseFloat(reserve) * 100),
          pointTtl: Math.round(parseFloat(ttl) * 86400),
        })], [kp]))}>
        {busy === 'terms' ? 'confirming…' : 'update terms'}
      </button>
    </Card>
  );
}

function OfferCard({ busy, run, client, kp }: any) {
  const { net } = useNet();
  const self = client.merchant(kp.publicKey).toBase58();
  const issuers = (net?.merchants ?? []).filter((m: any) => m.address !== self);
  const [issuer, setIssuer] = useState('');
  const [rate, setRate] = useState('110');
  const [cap, setCap] = useState('500');
  const [days, setDays] = useState('30');
  const myOffers = (net?.offers ?? []).filter((o: any) => o.acceptorStr === self);
  return (
    <Card title="acceptance offers" sub="Bid to honour another merchant's points. Above 100% you buy footfall; below it you discount their credit. No permission from them.">
      <Field label="honour issuer">
        <select value={issuer} onChange={(e) => setIssuer(e.target.value)}>
          <option value="">select a merchant…</option>
          {issuers.map((m: any) => <option key={m.address} value={m.address}>{displayName(m).label} · {shortAddr(m.address, 4)}</option>)}
        </select>
      </Field>
      <div className="formgrid">
        <Field label="rate (%)"><input value={rate} onChange={(e) => setRate(e.target.value)} /></Field>
        <Field label="capacity ($ face)"><input value={cap} onChange={(e) => setCap(e.target.value)} /></Field>
      </div>
      <Field label="expires (days)"><input value={days} onChange={(e) => setDays(e.target.value)} /></Field>
      <div className="row gap-8">
        <button className="btn btn-amber grow" disabled={!!busy || !issuer}
          onClick={() => run('offer', () => send(client, [client.postOffer({
            acceptorAuthority: kp.publicKey,
            issuerMerchant: new PublicKey(issuer),
            rateBps: Math.round(parseFloat(rate) * 100),
            capacity: Math.round(parseFloat(cap) * 1e6),
            expiresAt: Math.floor(Date.now() / 1000) + Math.round(parseFloat(days) * 86400),
          })], [kp]))}>
          {busy === 'offer' ? 'confirming…' : 'post offer'}
        </button>
        <button className="btn grow" disabled={!!busy || !issuer}
          onClick={() => run('cancel', () => send(client, [client.cancelOffer({ acceptorAuthority: kp.publicKey, issuerMerchant: new PublicKey(issuer) })], [kp]))}>
          cancel offer
        </button>
      </div>
      {myOffers.length > 0 && (
        <div style={{ marginTop: 12 }}>
          <div className="eyebrow" style={{ marginBottom: 6 }}>your live offers</div>
          {myOffers.map((o: any) => (
            <div key={o.issuerStr} className="kv"><span className="k">{shortAddr(o.issuerStr, 4)}</span><span className="v">{bps(o.rateBps)} · cap {usd(o.capacity)} · used {usd(o.consumed)}</span></div>
          ))}
        </div>
      )}
    </Card>
  );
}

/* ---------- bits ---------- */

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return <div className="field"><label>{label}</label>{children}</div>;
}
function Book({ k, v, accent }: { k: string; v: string; accent?: string }) {
  return (
    <div>
      <div className="mono" style={{ fontSize: 9, letterSpacing: '0.12em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>{k}</div>
      <div className="mono" style={{ fontSize: 16, fontWeight: 600, color: accent ?? 'var(--ink)', marginTop: 3 }}>{v}</div>
    </div>
  );
}
function MiniStat({ k, v, accent }: { k: string; v: string; accent?: string }) {
  return (
    <div className="col" style={{ alignItems: 'flex-end', gap: 1 }}>
      <span className="mono" style={{ fontSize: 9, letterSpacing: '0.12em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>{k}</span>
      <span className="mono" style={{ fontSize: 15, fontWeight: 600, color: accent ?? 'var(--ink)' }}>{v}</span>
    </div>
  );
}
function Empty() {
  return (
    <div className="panel" style={{ padding: 32, maxWidth: 560, margin: '40px auto', textAlign: 'center' }}>
      <div className="brand-mark" style={{ fontSize: 34, marginBottom: 12 }}>▸ RUN A MERCHANT</div>
      <p className="mono dim" style={{ fontSize: 12.5, lineHeight: 1.7 }}>
        The console signs real devnet transactions with a dev key. Open the <b style={{ color: 'var(--amber)' }}>DEV KEY</b> menu
        (top right) to import a secret key or mint a burner, then fund it with free devnet SOL. Register a merchant,
        create its Token-2022 points mint, and post acceptance offers — all with SOL alone.
      </p>
    </div>
  );
}
