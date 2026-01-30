import { Entity, PrimaryGeneratedColumn, Column, CreateDateColumn, UpdateDateColumn, OneToMany, Index } from 'typeorm';
import { TwoFactorAuth } from './twoFactorAuth.entity';
import { ApiKey } from './apiKey.entity';

@Entity('users')
@Index(['email'], { unique: true })
export class User {
  @PrimaryGeneratedColumn('uuid')
  id!: string;

  @Column({ unique: true })
  email!: string;

  @Column({ select: false })
  passwordHash!: string;

  @Column({ type: 'varchar', length: 255, nullable: true })
  firstName?: string | null;

  @Column({ type: 'varchar', length: 255, nullable: true })
  lastName?: string | null;

  // Publisher association - links user to their publisher account
  @Column({ type: 'uuid', nullable: true, name: 'publisher_id' })
  publisherId?: string | null;

  // RBAC role for access control
  @Column({ type: 'varchar', length: 50, default: 'publisher' })
  role!: 'admin' | 'publisher' | 'readonly';

  @CreateDateColumn()
  createdAt!: Date;

  @UpdateDateColumn()
  updatedAt!: Date;

  @OneToMany(() => TwoFactorAuth, (tfa) => tfa.user)
  twoFactorAuth!: TwoFactorAuth[];

  @OneToMany(() => ApiKey, (apiKey) => apiKey.user)
  apiKeys!: ApiKey[];
}
