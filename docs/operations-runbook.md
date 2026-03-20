# Operations Runbook

## Starting Paper Trading

1. Set environment variables:
   ```bash
   export BINANCE_API_KEY=your_testnet_key
   export BINANCE_API_SECRET=your_testnet_secret
   ```
2. Run health check: `opentrade doctor`
3. Start paper mode: `opentrade paper`
4. Monitor logs for signal generation and simulated fills

## Going Live (Checklist)

- [ ] Paper traded for at least 3 months
- [ ] Backtest results reviewed and understood (they do NOT guarantee live performance)
- [ ] Risk limits reviewed and appropriate for capital
- [ ] API key has only trading permissions (no withdrawal)
- [ ] Using testnet first to verify connectivity
- [ ] Emergency procedures documented and tested
- [ ] Monitoring and alerting configured
- [ ] Start with minimal capital
- [ ] Run: `opentrade live --confirm`

## Emergency Procedures

### Flatten All Positions
```bash
opentrade flatten --confirm
```

### Cancel All Orders
```bash
opentrade cancel-all --confirm
```

### Kill Switch (Manual)
- Set config `mode: paper` and restart
- Or press Ctrl+C for graceful shutdown

## Incident Response

1. **Detect**: Monitor logs, metrics, and alerts
2. **Contain**: Flatten positions if needed
3. **Investigate**: Check trade journal, logs, and risk snapshots
4. **Remediate**: Fix root cause
5. **Review**: Post-incident analysis

## Daily Operations

- Check daily PnL summary
- Verify no kill switches are triggered
- Review open positions and exposure
- Check for stale data warnings
- Monitor exchange connectivity

## Monitoring

- **Prometheus**: http://localhost:9090
- **Grafana**: http://localhost:3000
- Key metrics: `opentrade_portfolio_equity_usd`, `opentrade_drawdown_pct`, `opentrade_trades_total`
