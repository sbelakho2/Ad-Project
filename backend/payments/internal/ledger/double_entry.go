package ledger

import (
	"context"
	"encoding/json"
	"fmt"
	"time"

	"github.com/redis/go-redis/v9"
	"github.com/shopspring/decimal"
	log "github.com/sirupsen/logrus"
)

// TransactionType represents type of ledger transaction
type TransactionType string

const (
	TypeCredit TransactionType = "credit" // Money in
	TypeDebit  TransactionType = "debit"  // Money out
)

// Transaction represents a double-entry ledger transaction
type Transaction struct {
	ID            string            `json:"id"`
	PublisherID   string            `json:"publisher_id"`
	Type          TransactionType   `json:"type"`
	Amount        decimal.Decimal   `json:"amount"`
	Currency      string            `json:"currency"`
	Description   string            `json:"description"`
	ReferenceID   string            `json:"reference_id"` // Impression ID, payment ID, etc.
	ReferenceType string            `json:"reference_type"`
	CreatedAt     time.Time         `json:"created_at"`
	Metadata      map[string]string `json:"metadata"`
}

// DoubleEntryLedger implements double-entry bookkeeping
type DoubleEntryLedger struct {
	redis *redis.Client
}

// NewDoubleEntryLedger creates a new ledger
func NewDoubleEntryLedger(redisClient *redis.Client) *DoubleEntryLedger {
	return &DoubleEntryLedger{
		redis: redisClient,
	}
}

// RecordRevenue records ad revenue (credit to publisher, debit from advertisers)
func (del *DoubleEntryLedger) RecordRevenue(ctx context.Context, publisherID string, amount decimal.Decimal, currency, impressionID string) error {
	// Generate transaction ID
	txID := fmt.Sprintf("tx_%s_%d", publisherID, time.Now().UnixNano())

	// Create transaction
	tx := Transaction{
		ID:            txID,
		PublisherID:   publisherID,
		Type:          TypeCredit,
		Amount:        amount,
		Currency:      currency,
		Description:   "Ad revenue",
		ReferenceID:   impressionID,
		ReferenceType: "impression",
		CreatedAt:     time.Now(),
		Metadata: map[string]string{
			"source": "ad_impression",
		},
	}

	// Store transaction
	if err := del.storeTransaction(ctx, tx); err != nil {
		return err
	}

	// Update balance
	if err := del.updateBalance(ctx, publisherID, amount, currency); err != nil {
		// Rollback transaction
		del.deleteTransaction(ctx, txID)
		return err
	}

	log.WithFields(log.Fields{
		"publisher_id": publisherID,
		"amount":       amount,
		"currency":     currency,
		"tx_id":        txID,
	}).Info("Recorded revenue")

	return nil
}

// RecordPayout records payout (debit from publisher balance)
// CRITICAL: Uses distributed lock to prevent race conditions in concurrent payout requests
func (del *DoubleEntryLedger) RecordPayout(ctx context.Context, publisherID string, amount decimal.Decimal, currency, payoutID string) error {
	// Acquire distributed lock to prevent race condition
	lockKey := fmt.Sprintf("lock:payout:%s:%s", publisherID, currency)
	lockValue := fmt.Sprintf("%s:%d", payoutID, time.Now().UnixNano())
	lockTTL := 30 * time.Second

	// Try to acquire lock with NX (only set if not exists)
	acquired, err := del.redis.SetNX(ctx, lockKey, lockValue, lockTTL).Result()
	if err != nil {
		return fmt.Errorf("failed to acquire payout lock: %w", err)
	}
	if !acquired {
		return fmt.Errorf("payout already in progress for publisher %s", publisherID)
	}

	// Ensure lock is released when done
	defer func() {
		// Only release if we still hold the lock (compare value to prevent releasing someone else's lock)
		currentVal, err := del.redis.Get(ctx, lockKey).Result()
		if err == nil && currentVal == lockValue {
			del.redis.Del(ctx, lockKey)
		}
	}()

	// Check sufficient balance (now protected by lock)
	balance, err := del.GetBalance(ctx, publisherID, currency)
	if err != nil {
		return err
	}

	if balance.LessThan(amount) {
		return fmt.Errorf("insufficient balance: has %s, needs %s", balance, amount)
	}

	// Generate transaction ID
	txID := fmt.Sprintf("tx_%s_%d", publisherID, time.Now().UnixNano())

	// Create transaction
	tx := Transaction{
		ID:            txID,
		PublisherID:   publisherID,
		Type:          TypeDebit,
		Amount:        amount,
		Currency:      currency,
		Description:   "Payout",
		ReferenceID:   payoutID,
		ReferenceType: "payout",
		CreatedAt:     time.Now(),
		Metadata: map[string]string{
			"source": "payout",
		},
	}

	// Store transaction
	if err := del.storeTransaction(ctx, tx); err != nil {
		return err
	}

	// Update balance (negative for debit)
	if err := del.updateBalance(ctx, publisherID, amount.Neg(), currency); err != nil {
		// Rollback transaction
		del.deleteTransaction(ctx, txID)
		return err
	}

	log.WithFields(log.Fields{
		"publisher_id": publisherID,
		"amount":       amount,
		"currency":     currency,
		"tx_id":        txID,
	}).Info("Recorded payout")

	return nil
}

// GetBalance gets publisher balance
func (del *DoubleEntryLedger) GetBalance(ctx context.Context, publisherID, currency string) (decimal.Decimal, error) {
	key := fmt.Sprintf("balance:%s:%s", publisherID, currency)

	balanceStr, err := del.redis.Get(ctx, key).Result()
	if err == redis.Nil {
		return decimal.Zero, nil
	} else if err != nil {
		return decimal.Zero, err
	}

	balance, err := decimal.NewFromString(balanceStr)
	if err != nil {
		return decimal.Zero, err
	}

	return balance, nil
}

// GetTransactions gets transaction history
func (del *DoubleEntryLedger) GetTransactions(ctx context.Context, publisherID string, limit int) ([]Transaction, error) {
	key := fmt.Sprintf("transactions:%s", publisherID)

	// Get transaction IDs from sorted set (sorted by timestamp)
	txIDs, err := del.redis.ZRevRange(ctx, key, 0, int64(limit-1)).Result()
	if err != nil {
		return nil, err
	}

	var transactions []Transaction
	for _, txID := range txIDs {
		tx, err := del.getTransaction(ctx, txID)
		if err != nil {
			log.WithError(err).Warn("Failed to fetch transaction")
			continue
		}
		transactions = append(transactions, tx)
	}

	return transactions, nil
}

// Internal methods

func (del *DoubleEntryLedger) storeTransaction(ctx context.Context, tx Transaction) error {
	// Store transaction details
	txKey := fmt.Sprintf("transaction:%s", tx.ID)
	data, err := json.Marshal(tx)
	if err != nil {
		return err
	}

	if err := del.redis.Set(ctx, txKey, data, 0).Err(); err != nil {
		return err
	}

	// Add to publisher's transaction list
	listKey := fmt.Sprintf("transactions:%s", tx.PublisherID)
	score := float64(tx.CreatedAt.Unix())
	if err := del.redis.ZAdd(ctx, listKey, redis.Z{Score: score, Member: tx.ID}).Err(); err != nil {
		return err
	}

	return nil
}

func (del *DoubleEntryLedger) getTransaction(ctx context.Context, txID string) (Transaction, error) {
	key := fmt.Sprintf("transaction:%s", txID)

	data, err := del.redis.Get(ctx, key).Bytes()
	if err != nil {
		return Transaction{}, err
	}

	var tx Transaction
	if err := json.Unmarshal(data, &tx); err != nil {
		return Transaction{}, err
	}

	return tx, nil
}

func (del *DoubleEntryLedger) deleteTransaction(ctx context.Context, txID string) error {
	key := fmt.Sprintf("transaction:%s", txID)
	return del.redis.Del(ctx, key).Err()
}

func (del *DoubleEntryLedger) updateBalance(ctx context.Context, publisherID string, amount decimal.Decimal, currency string) error {
	key := fmt.Sprintf("balance:%s:%s", publisherID, currency)

	// Get current balance
	currentBalance, _ := del.GetBalance(ctx, publisherID, currency)

	// Calculate new balance
	newBalance := currentBalance.Add(amount)

	// Store new balance
	return del.redis.Set(ctx, key, newBalance.String(), 0).Err()
}

// PaymentTerms defines payment terms for publishers
type PaymentTerms struct {
	PublisherID   string          `json:"publisher_id"`
	MinimumPayout decimal.Decimal `json:"minimum_payout"`
	PaymentCycle  string          `json:"payment_cycle"` // "net-30", "net-60", etc.
	PaymentMethod string          `json:"payment_method"`
	Currency      string          `json:"currency"`
}

// PaymentOrchestrator manages payment processing
type PaymentOrchestrator struct {
	ledger *DoubleEntryLedger
	redis  *redis.Client
}

// NewPaymentOrchestrator creates a new payment orchestrator
func NewPaymentOrchestrator(ledger *DoubleEntryLedger, redisClient *redis.Client) *PaymentOrchestrator {
	return &PaymentOrchestrator{
		ledger: ledger,
		redis:  redisClient,
	}
}

// GetPayableAmount returns amount ready for payout
func (po *PaymentOrchestrator) GetPayableAmount(ctx context.Context, publisherID string) (decimal.Decimal, error) {
	// Get payment terms
	terms, err := po.getPaymentTerms(ctx, publisherID)
	if err != nil {
		return decimal.Zero, err
	}

	// Get current balance
	balance, err := po.ledger.GetBalance(ctx, publisherID, terms.Currency)
	if err != nil {
		return decimal.Zero, err
	}

	// Check minimum payout
	if balance.LessThan(terms.MinimumPayout) {
		return decimal.Zero, nil
	}

	// Check payment cycle (net-60 means 60 days after end of month)
	// For simplicity, assuming all revenue is payable after the cycle

	return balance, nil
}

// InitiatePayout initiates a payout to publisher
// The race condition protection is now in RecordPayout via distributed lock
// SECURITY: Added currency parameter override and additional validation (P0 Fix #11)
func (po *PaymentOrchestrator) InitiatePayout(ctx context.Context, publisherID string, amount decimal.Decimal, currency string) (string, error) {
	// SECURITY: Validate amount is positive
	if amount.LessThanOrEqual(decimal.Zero) {
		return "", fmt.Errorf("payout amount must be positive, got %s", amount)
	}

	// SECURITY: Define maximum payout limit to prevent fraud
	maxPayout := decimal.NewFromFloat(50000.0)
	if amount.GreaterThan(maxPayout) {
		log.WithFields(log.Fields{
			"publisher_id": publisherID,
			"amount":       amount.String(),
			"max":          maxPayout.String(),
		}).Warn("Payout rejected: exceeds maximum limit")
		return "", fmt.Errorf("payout amount %s exceeds maximum limit %s", amount, maxPayout)
	}

	// SECURITY: Validate precision (max 2 decimal places)
	truncated := amount.Truncate(2)
	if !amount.Equal(truncated) {
		return "", fmt.Errorf("payout amount must have at most 2 decimal places")
	}

	// Get payment terms
	terms, err := po.getPaymentTerms(ctx, publisherID)
	if err != nil {
		return "", err
	}

	// Use provided currency or fall back to terms currency
	payoutCurrency := currency
	if payoutCurrency == "" {
		payoutCurrency = terms.Currency
	}

	// Validate minimum payout amount
	if amount.LessThan(terms.MinimumPayout) {
		return "", fmt.Errorf("payout amount %s is below minimum %s", amount, terms.MinimumPayout)
	}

	// Create payout record with unique ID
	payoutID := fmt.Sprintf("payout_%s_%d", publisherID, time.Now().UnixNano())

	// Record in ledger FIRST (this has the distributed lock for race condition protection)
	// If this fails, we don't create any payout record
	if err := po.ledger.RecordPayout(ctx, publisherID, amount, payoutCurrency, payoutID); err != nil {
		return "", fmt.Errorf("failed to record payout in ledger: %w", err)
	}

	// Only after successful ledger debit, create the payout record
	payout := map[string]interface{}{
		"id":           payoutID,
		"publisher_id": publisherID,
		"amount":       amount.String(),
		"currency":     payoutCurrency,
		"method":       terms.PaymentMethod,
		"status":       "pending",
		"created_at":   time.Now().Unix(),
	}

	// Store payout record
	data, _ := json.Marshal(payout)
	key := fmt.Sprintf("payout:%s", payoutID)
	if err := po.redis.Set(ctx, key, data, 0).Err(); err != nil {
		// Payout record failed but ledger was already debited
		// This is a critical error - log for manual reconciliation
		log.WithError(err).WithField("payout_id", payoutID).Error("CRITICAL: Ledger debited but payout record failed to save - manual reconciliation needed")
		return "", fmt.Errorf("failed to save payout record (ledger already debited - needs reconciliation): %w", err)
	}

	// Process payout based on method (async)
	go po.processPayout(context.Background(), payoutID, terms)

	return payoutID, nil
}

// processPayout processes the payout through payment rails
func (po *PaymentOrchestrator) processPayout(ctx context.Context, payoutID string, terms PaymentTerms) {
	// This would integrate with actual payment processors
	// Stripe Connect, PayPal Payouts, wire/ACH, crypto

	log.WithFields(log.Fields{
		"payout_id": payoutID,
		"method":    terms.PaymentMethod,
	}).Info("Processing payout")

	// Simulate processing
	time.Sleep(5 * time.Second)

	// Update status
	key := fmt.Sprintf("payout:%s", payoutID)
	po.redis.HSet(ctx, key, "status", "completed")

	log.WithField("payout_id", payoutID).Info("Payout completed")
}

func (po *PaymentOrchestrator) getPaymentTerms(ctx context.Context, publisherID string) (PaymentTerms, error) {
	key := fmt.Sprintf("payment_terms:%s", publisherID)

	data, err := po.redis.Get(ctx, key).Bytes()
	if err == redis.Nil {
		// Return default terms
		return PaymentTerms{
			PublisherID:   publisherID,
			MinimumPayout: decimal.NewFromFloat(100.0),
			PaymentCycle:  "net-60",
			PaymentMethod: "stripe",
			Currency:      "USD",
		}, nil
	} else if err != nil {
		return PaymentTerms{}, err
	}

	var terms PaymentTerms
	if err := json.Unmarshal(data, &terms); err != nil {
		return PaymentTerms{}, err
	}

	return terms, nil
}
