import { Entity, PrimaryGeneratedColumn, Column, ManyToOne, Index, JoinColumn } from 'typeorm';
import { User } from './user.entity';

@Entity('two_factor_auth')
@Index(['user', 'enabled'])
export class TwoFactorAuth {
  @PrimaryGeneratedColumn('uuid')
  id!: string;

  @ManyToOne(() => User, (user: User) => user.twoFactorAuth, { onDelete: 'CASCADE' })
  @JoinColumn()
  user!: User;

  // Deprecated: plaintext secret (kept nullable for backward compatibility)
  // Explicitly set column type for Postgres to avoid reflect-metadata inferring Object for union types
  @Column({ type: 'text', nullable: true })
  secret!: string | null;

  // Encrypted TOTP secret (AES-GCM base64 payload as JSON string)
  @Column({ type: 'text', nullable: true })
  secretCiphertext!: string | null;

  @Column({ default: false })
  enabled!: boolean;

  @Column({ type: 'simple-array', nullable: true })
  backupCodes!: string[]; // Store hashed backup codes
}
