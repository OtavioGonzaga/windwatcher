local now = ARGV[3]
for i=1,64 do
    local a = first_due('pending',now)
    local b = first_due('leased',now)
    if not a and not b then return {} end
    local m = a
    local old = 'pending'
    if b and (not a or b < a) then m = b; old = 'leased' end
    local id = job_id(m)
    local k = hash(id)
    if redis.call('HGET',k,'status') ~= old then corrupt('inconsistent-index') end
    local field = old == 'pending' and 'available_at' or 'lease_expires_at'
    if m ~= stored_time(k,field) .. ':' .. id then corrupt('inconsistent-index') end
    local attempts, maximum = counters(k)
    if attempts >= maximum then
        if old ~= 'leased' then corrupt('exhausted-pending') end
        exhausted(k,id,now)
    else
        -- Validate metadata before mutation; payload remains opaque.
        member(k,id)
        remove_due(k,id,old)
        move(k,id,old,'leased')
        redis.call('HSET',k,'attempts',tostring(attempts+1),'lease_id',ARGV[1],'lease_owner',ARGV[2],'lease_expires_at',ARGV[4])
        redis.call('ZADD',due('leased'),0,ARGV[4] .. ':' .. id)
        return redis.call('HGETALL',k)
    end
end
return {}
